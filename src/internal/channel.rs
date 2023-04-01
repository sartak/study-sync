use async_trait::async_trait;
use log::info;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout_at;
use tokio::time::Instant;

pub enum Action {
    Continue,
    Halt,
    Retry,
}

#[async_trait]
pub trait PriorityRetryChannel {
    type Event: std::fmt::Debug + Send + Sync;

    fn is_online(&self) -> bool;
    fn is_high_priority(&self, event: &Self::Event) -> bool;
    async fn handle(&mut self, event: &Self::Event) -> Action;

    async fn run<'a>(&mut self, mut rx: mpsc::UnboundedReceiver<Self::Event>) {
        let mut retry_deadline = None;
        let mut buffer = VecDeque::new();
        let mut priority_event = None;
        let mut priority_retry = None;
        let mut normal_retry = None;

        let online_secs = 5;
        let offline_secs = 30;

        loop {
            if let Some(event) = priority_event {
                match self.handle(&event).await {
                    Action::Continue => {
                        priority_retry = None;
                        priority_event = None;
                    }
                    Action::Halt => break,
                    Action::Retry => {
                        let retries = if let Some(r) = priority_retry {
                            let r = r + 1;
                            priority_retry = Some(r);
                            if r > 5 {
                                5
                            } else {
                                r
                            }
                        } else {
                            priority_retry = Some(1);
                            1
                        };

                        let wait = retries
                            * if self.is_online() {
                                online_secs
                            } else {
                                offline_secs
                            };

                        info!("Waiting for {wait}s before retrying");
                        tokio::time::sleep(Duration::from_secs(wait)).await;
                        priority_event = Some(event);
                        continue;
                    }
                }
            }

            // If the buffer is empty, block until we get an event
            let event = if buffer.is_empty() {
                rx.recv().await
            // Otherwise we have events to process. First let's see if we have
            // a deadline to wait for; if so then we'll block on the channel
            // until the deadline
            } else if let Some((online_deadline, offline_deadline)) = retry_deadline {
                let deadline = if self.is_online() {
                    online_deadline
                } else {
                    offline_deadline
                };
                match timeout_at(deadline, rx.recv()).await {
                    Ok(event) => event,
                    Err(_) => {
                        retry_deadline = None;
                        None
                    }
                }
            // Otherwise we have an event to process but no deadline to wait for
            // so just quickly check the channel (which will almost certainly be
            // empty) then proceed to processing events
            } else {
                match rx.try_recv() {
                    Ok(e) => Some(e),
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                    _ => None,
                }
            };

            if let Some(event) = event {
                if self.is_high_priority(&event) {
                    match self.handle(&event).await {
                        Action::Continue => priority_retry = None,

                        Action::Halt => break,

                        Action::Retry => {
                            let retries = if let Some(r) = priority_retry {
                                let r = r + 1;
                                priority_retry = Some(r);
                                if r > 5 {
                                    5
                                } else {
                                    r
                                }
                            } else {
                                priority_retry = Some(1);
                                1
                            };

                            let wait = retries
                                * if self.is_online() {
                                    online_secs
                                } else {
                                    offline_secs
                                };

                            info!("Waiting for {wait}s before retrying");
                            tokio::time::sleep(Duration::from_secs(wait)).await;
                            priority_event = Some(event);
                        }
                    }
                } else {
                    buffer.push_back(event);
                }
            } else if let Some(event) = buffer.pop_front() {
                match self.handle(&event).await {
                    Action::Continue => normal_retry = None,

                    Action::Halt => break,

                    Action::Retry => {
                        buffer.push_front(event);
                        let now = Instant::now();

                        let retries = if let Some(r) = normal_retry {
                            let r = r + 1;
                            normal_retry = Some(r);
                            if r > 5 {
                                5
                            } else {
                                r
                            }
                        } else {
                            normal_retry = Some(1);
                            1
                        };

                        let online_secs = retries * online_secs;
                        let offline_secs = retries * offline_secs;

                        let (a, b) = if self.is_online() {
                            (online_secs, offline_secs)
                        } else {
                            (offline_secs, online_secs)
                        };
                        info!("Waiting for {a}s (or possibly {b}s) before retrying");

                        retry_deadline = Some((
                            now + Duration::from_secs(online_secs),
                            now + Duration::from_secs(offline_secs),
                        ))
                    }
                }
            }
        }
    }
}
