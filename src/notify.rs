use anyhow::Result;
use log::{error, info};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Event {
    Success(String),
    Error(String),
    Emergency(String),
    StartShutdown,
}

pub struct NotifyPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Notify {
    rx: mpsc::UnboundedReceiver<Event>,
    led_path: PathBuf,
}

pub fn launch() -> (NotifyPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (NotifyPre { rx }, tx)
}

impl NotifyPre {
    pub async fn start(self, led_path: PathBuf) -> Result<()> {
        let notify = Notify {
            rx: self.rx,
            led_path,
        };
        notify.start().await
    }
}

impl Notify {
    pub async fn start(mut self) -> Result<()> {
        while let Some(event) = self.rx.recv().await {
            info!("Handling {event:?}");
            match event {
                Event::Success(_) => {
                    self.red().await;
                    self.wait(500).await;
                    self.green().await;

                    self.wait(500).await;
                }

                Event::Error(_) => {
                    for _ in 1..3 {
                        self.red().await;
                        self.wait(250).await;
                        self.green().await;
                        self.wait(250).await;
                    }

                    self.wait(250).await;
                }

                Event::Emergency(_) => {
                    for _ in 1..10 {
                        self.red().await;
                        self.wait(100).await;
                        self.green().await;
                        self.wait(100).await;
                    }

                    self.wait(900).await;
                }

                Event::StartShutdown => break,
            }
        }

        info!("notify gracefully shut down");
        Ok(())
    }

    async fn wait(&self, ms: u64) {
        tokio::time::sleep(tokio::time::Duration::from_millis(ms)).await;
    }

    async fn red(&self) {
        if let Err(e) = self.change_led(true).await {
            error!("{e:?}");
        }
    }

    async fn green(&self) {
        if let Err(e) = self.change_led(false).await {
            error!("{e:?}");
        }
    }

    async fn change_led(&self, on: bool) -> Result<()> {
        let mut file = File::create(&self.led_path).await?;
        let bytes = if on { b"0" } else { b"1" };
        file.write_all(bytes).await?;
        Ok(())
    }
}

pub trait Notifier {
    fn notify_target(&self) -> &str;

    fn notify_tx(&self) -> &mpsc::UnboundedSender<Event>;

    fn notify_success(&self, message: String) {
        info!(target: self.notify_target(), "Success: {message:?}");

        if let Err(e) = self.notify_tx().send(Event::Success(message.clone())) {
            error!(target: self.notify_target(), "Could not send success {message:?} to notify: {e:?}");
        }
    }

    fn notify_error(&self, message: String) {
        error!(target: self.notify_target(), "Error: {message:?}");

        if let Err(e) = self.notify_tx().send(Event::Error(message.clone())) {
            error!(target: self.notify_target(), "Could not send error {message:?} to notify: {e:?}");
        }
    }

    fn notify_emergency(&self, message: String) {
        error!(target: self.notify_target(), "Emergency: {message:?}");

        if let Err(e) = self.notify_tx().send(Event::Emergency(message.clone())) {
            error!(target: self.notify_target(), "Could not send emergency {message:?} to notify: {e:?}");
        }
    }
}
