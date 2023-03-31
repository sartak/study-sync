use crate::{
    notify::{self, Notifier},
    orchestrator::{self, Language},
    screenshots::Online,
};
use anyhow::{anyhow, Result};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

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
    rx: mpsc::UnboundedReceiver<Event>,
    orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
    notify_tx: mpsc::UnboundedSender<notify::Event>,
    intake_url: String,
    play_to_intake: HashMap<i64, String>,
    buffer: VecDeque<Event>,
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
            rx: self.rx,
            orchestrator_tx,
            notify_tx,
            intake_url,
            play_to_intake: HashMap::new(),
            buffer: VecDeque::new(),
            is_online,
        };
        intake.start().await
    }
}

impl Intake {
    pub async fn start(mut self) -> Result<()> {
        loop {
            // if we have a buffer, then we want to just check on the channel and continue
            // otherwise block
            let event = if self.buffer.is_empty() {
                self.rx.recv().await
            } else {
                match self.rx.try_recv() {
                    Ok(e) => Some(e),
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                    _ => None,
                }
            };

            if let Some(event) = event {
                info!("Handling event {event:?}");
                match event {
                    Event::StartShutdown => break,

                    Event::IsOnline(online) => self.is_online = online,

                    _ => self.buffer.push_back(event),
                }
            } else if let Some(event) = self.buffer.pop_front() {
                match &event {
                    Event::PreviousGame { play_id, intake_id } => {
                        self.play_to_intake.insert(*play_id, intake_id.clone());
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
                                self.buffer.push_front(event);
                                info!("Sleeping for 5s before trying again");
                                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                                continue;
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
                            continue;
                        }
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
                                self.buffer.push_front(event);
                                info!("Sleeping for 5s before trying again");
                                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                                continue;
                            }
                        };

                        self.play_to_intake.remove(play_id);

                        let msg = orchestrator::Event::IntakeEnded {
                            play_id: *play_id,
                            submitted_end,
                        };
                        if let Err(e) = self.orchestrator_tx.send(msg) {
                            self.notify_error(&format!("Could not send to orchestrator: {e:?}"));
                            continue;
                        }
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
                            let submitted_end = match self.finish_intake(intake_id, *end_time).await
                            {
                                Ok(e) => e,
                                Err(e) => {
                                    error!("Could not finish intake: {e:?}");
                                    self.buffer.push_front(event);
                                    info!("Sleeping for 5s before trying again");
                                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                                    continue;
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
                                    self.buffer.push_front(event);
                                    info!("Sleeping for 5s before trying again");
                                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                                    continue;
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
                            continue;
                        }
                    }

                    Event::IsOnline(_) => unreachable!(),

                    Event::StartShutdown => unreachable!(),
                }
            }
        }

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
            self.request(reqwest::Method::POST, request).await?;
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

        let (submitted, _) = self.request(reqwest::Method::PATCH, request).await?;
        Ok(submitted)
    }

    async fn request<R>(
        &self,
        method: reqwest::Method,
        request: R,
    ) -> Result<(u64, IntakeResponseObject)>
    where
        R: Serialize + std::fmt::Debug,
    {
        let url = &self.intake_url;

        let submitted = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let builder = reqwest::ClientBuilder::new().timeout(Duration::from_secs(10));
        let client = builder.build()?;

        let res = client
            .request(method.clone(), url)
            .json(&request)
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!(
                "Error {method:?}ing {url:?} with {request:?}: got status code {}",
                res.status()
            ));
        }

        match res.json().await? {
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
    fn notify_target(&self) -> &str {
        "study_sync::intake"
    }

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
