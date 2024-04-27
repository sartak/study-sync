use crate::{
    internal::{
        channel::{Action, PriorityRetryChannel},
        notifier::Notifier,
        online::Online,
        requester::Requester,
    },
    notify,
    orchestrator::{self, Language},
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

#[derive(Debug)]
pub enum Event {
    PreviousGame {
        play_id: i64,
        intake_id: String,
    },
    SubmitStarted {
        play_id: i64,
        game_label: String,
        language: Language,
        start_time: u64,
    },
    SubmitEnded {
        play_id: i64,
        intake_id: String,
        end_time: u64,
    },
    SubmitFull {
        play_id: i64,
        game_label: String,
        language: Language,
        start_time: u64,
        end_time: u64,
    },
    IsOnline(bool),
    ForceSync,
    StartShutdown,
}

#[derive(Debug, Deserialize)]
struct IntakeResponse {
    message: Option<String>,
    error: Option<String>,
    object: Option<IntakeResponseObject>,
}

#[derive(Debug, Deserialize)]
struct IntakeResponseObject {
    rowid: String,
}

pub struct IntakePre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Intake {
    orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
    notify_tx: mpsc::UnboundedSender<notify::Event>,
    intake_url: String,
    play_to_intake: HashMap<i64, String>,
    is_online: bool,
}

pub fn prepare() -> (IntakePre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (IntakePre { rx }, tx)
}

impl IntakePre {
    pub async fn start(
        self,
        orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
        notify_tx: mpsc::UnboundedSender<notify::Event>,
        intake_url: String,
        is_online: bool,
    ) -> Result<()> {
        let intake = Intake {
            orchestrator_tx,
            notify_tx,
            intake_url,
            play_to_intake: HashMap::new(),
            is_online,
        };
        intake.start(self.rx).await
    }
}

impl Intake {
    pub async fn start(mut self, rx: mpsc::UnboundedReceiver<Event>) -> Result<()> {
        self.run(rx).await;
        info!("intake gracefully shut down");
        Ok(())
    }

    async fn create_intake(
        &self,
        game_label: &str,
        language: &Language,
        start_time: u64,
        end_time: Option<u64>,
    ) -> Result<(String, u64)> {
        #[derive(Debug, Serialize)]
        struct Request<'a> {
            #[serde(rename = "startTime")]
            start_time: u64,
            #[serde(rename = "endTime")]
            end_time: Option<u64>,
            #[serde(rename = "game")]
            game_label: &'a str,
            language: &'a str,
        }

        let request = Request {
            start_time,
            end_time,
            game_label,
            language: language.intake_str(),
        };

        let (submitted, IntakeResponseObject { rowid }) =
            self.submit(reqwest::Method::POST, request).await?;
        Ok((rowid, submitted))
    }

    async fn finish_intake(&self, intake_id: &str, end_time: u64) -> Result<u64> {
        #[derive(Debug, Serialize)]
        struct Request<'a> {
            rowid: &'a str,
            #[serde(rename = "endTime")]
            end_time: u64,
        }

        let request = Request {
            rowid: intake_id,
            end_time,
        };

        let (submitted, _) = self.submit(reqwest::Method::PATCH, request).await?;
        Ok(submitted)
    }

    async fn submit<R>(
        &self,
        method: reqwest::Method,
        request: R,
    ) -> Result<(u64, IntakeResponseObject)>
    where
        R: Serialize + std::fmt::Debug + Send + Sync,
    {
        let url = &self.intake_url;

        let submitted = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        match self.request(url, &method, request).await? {
            IntakeResponse {
                error: Some(error), ..
            } => Err(anyhow!("Error {method:?}ing {url:?} from server: {error}")),
            IntakeResponse {
                message: Some(message),
                object: Some(obj),
                ..
            } => {
                info!("Success {method:?}ing {url:?}: {message}");
                Ok((submitted, obj))
            }
            res => Err(anyhow!(
                "Error pattern-matching {method:?} {url:?} response: {res:?}"
            )),
        }
    }
}

impl Language {
    fn intake_str(&self) -> &str {
        match self {
            Language::English => "English",
            Language::Japanese => "日本語",
            Language::Cantonese => "廣東話",
            Language::Other(lang) => {
                warn!("Mapping intake language {lang} to English");
                "English"
            }
        }
    }
}

impl Notifier for Intake {
    fn notify_tx(&self) -> &mpsc::UnboundedSender<notify::Event> {
        &self.notify_tx
    }
}

impl Online for Intake {
    fn orchestrator_tx(&self) -> &mpsc::UnboundedSender<orchestrator::Event> {
        &self.orchestrator_tx
    }

    fn is_online(&self) -> bool {
        self.is_online
    }
}

#[async_trait]
impl PriorityRetryChannel for Intake {
    type Event = Event;

    fn is_online(&self) -> bool {
        self.is_online
    }

    fn is_high_priority(&self, event: &Event) -> bool {
        match event {
            Event::StartShutdown => true,
            Event::IsOnline(_) => true,
            Event::ForceSync => true,

            Event::PreviousGame { .. } => false,
            Event::SubmitStarted { .. } => false,
            Event::SubmitEnded { .. } => false,
            Event::SubmitFull { .. } => false,
        }
    }

    async fn handle(&mut self, event: &Event) -> Action {
        info!("Handling event {event:?}");

        match event {
            Event::StartShutdown => Action::Halt,

            Event::IsOnline(online) => {
                self.is_online = *online;
                Action::Continue
            }

            Event::ForceSync => {
                self.is_online = true;
                Action::ResetTimeout
            }

            Event::PreviousGame { play_id, intake_id } => {
                self.play_to_intake.insert(*play_id, intake_id.clone());
                Action::Continue
            }

            Event::SubmitStarted {
                play_id,
                game_label,
                language,
                start_time,
            } => {
                let (intake_id, submitted_start) = match self
                    .create_intake(game_label, language, *start_time, None)
                    .await
                {
                    Ok((i, s)) => (i, s),
                    Err(e) => {
                        error!("Could not create intake: {e:?}");
                        return Action::Retry;
                    }
                };

                self.play_to_intake.insert(*play_id, intake_id.clone());
                let msg = orchestrator::Event::IntakeStarted {
                    play_id: *play_id,
                    intake_id,
                    submitted_start,
                };
                if let Err(e) = self.orchestrator_tx.send(msg) {
                    self.notify_error(&format!("Could not send to orchestrator: {e:?}"));
                }

                Action::Continue
            }

            Event::SubmitEnded {
                play_id,
                intake_id,
                end_time,
            } => {
                let submitted_end = match self.finish_intake(intake_id, *end_time).await {
                    Ok(e) => e,
                    Err(e) => {
                        error!("Could not finish intake: {e:?}");
                        return Action::Retry;
                    }
                };

                self.play_to_intake.remove(play_id);

                let msg = orchestrator::Event::IntakeEnded {
                    play_id: *play_id,
                    submitted_end,
                };
                if let Err(e) = self.orchestrator_tx.send(msg) {
                    self.notify_error(&format!("Could not send to orchestrator: {e:?}"));
                }

                Action::Continue
            }

            Event::SubmitFull {
                play_id,
                game_label,
                language,
                start_time,
                end_time,
            } => {
                let msg;
                if let Some(intake_id) = self.play_to_intake.get(play_id) {
                    let submitted_end = match self.finish_intake(intake_id, *end_time).await {
                        Ok(e) => e,
                        Err(e) => {
                            error!("Could not finish intake: {e:?}");
                            return Action::Retry;
                        }
                    };

                    self.play_to_intake.remove(play_id);

                    msg = orchestrator::Event::IntakeEnded {
                        play_id: *play_id,
                        submitted_end,
                    };
                } else {
                    let (intake_id, submitted) = match self
                        .create_intake(game_label, language, *start_time, Some(*end_time))
                        .await
                    {
                        Ok((i, s)) => (i, s),
                        Err(e) => {
                            error!("Could not create intake: {e:?}");
                            return Action::Retry;
                        }
                    };

                    msg = orchestrator::Event::IntakeFull {
                        play_id: *play_id,
                        intake_id,
                        submitted_start: submitted,
                        submitted_end: submitted,
                    };
                }

                if let Err(e) = self.orchestrator_tx.send(msg) {
                    self.notify_error(&format!("Could not send to orchestrator: {e:?}"));
                }

                Action::Continue
            }
        }
    }
}

impl Requester for Intake {}
