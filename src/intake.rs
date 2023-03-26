use crate::{game::Language, orchestrator};
use anyhow::Result;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Event {
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
}

#[derive(Debug, Serialize, Deserialize)]
struct IntakeResponse {
    message: Option<String>,
    error: Option<String>,
    object: Option<IntakeResponseObject>,
}

#[derive(Debug, Serialize, Deserialize)]
struct IntakeResponseObject {
    rowid: String,
}

pub struct IntakePre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Intake {
    rx: mpsc::UnboundedReceiver<Event>,
    orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
    intake_url: String,
    play_to_intake: HashMap<i64, String>,
}

pub fn launch() -> (IntakePre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    return (IntakePre { rx }, tx);
}

impl IntakePre {
    pub async fn start(
        self,
        orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
        intake_url: String,
    ) -> Result<()> {
        let intake = Intake {
            rx: self.rx,
            orchestrator_tx,
            intake_url,
            play_to_intake: HashMap::new(),
        };
        intake.start().await
    }
}

impl Intake {
    pub async fn start(mut self) -> Result<()> {
        while let Some(event) = self.rx.recv().await {
            info!("Handling event {event:?}");
            match event {
                Event::SubmitStarted {
                    play_id,
                    game_label,
                    language,
                    start_time,
                } => {
                    let (intake_id, submitted_start) = self
                        .create_intake(game_label, language, start_time, None)
                        .await;
                    self.play_to_intake.insert(play_id, intake_id.clone());
                    let event = orchestrator::Event::IntakeStarted {
                        play_id,
                        intake_id,
                        submitted_start,
                    };
                    if let Err(e) = self.orchestrator_tx.send(event) {
                        error!("Could not send to orchestrator: {e:?}");
                        continue;
                    }
                }
                Event::SubmitEnded {
                    play_id,
                    intake_id,
                    end_time,
                } => {
                    self.play_to_intake.remove(&play_id);

                    let submitted_end = self.finish_intake(intake_id, end_time).await;
                    let event = orchestrator::Event::IntakeEnded {
                        play_id,
                        submitted_end,
                    };
                    if let Err(e) = self.orchestrator_tx.send(event) {
                        error!("Could not send to orchestrator: {e:?}");
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
                    let event;
                    if let Some(intake_id) = self.play_to_intake.remove(&play_id) {
                        let submitted_end = self.finish_intake(intake_id, end_time).await;
                        event = orchestrator::Event::IntakeEnded {
                            play_id,
                            submitted_end,
                        };
                    } else {
                        let (intake_id, submitted_start) = self
                            .create_intake(game_label, language, start_time, Some(end_time))
                            .await;
                        event = orchestrator::Event::IntakeFull {
                            play_id,
                            intake_id,
                            submitted_start,
                            submitted_end: submitted_start,
                        };
                    }

                    if let Err(e) = self.orchestrator_tx.send(event) {
                        error!("Could not send to orchestrator: {e:?}");
                        continue;
                    }
                }
            }
        }
        Ok(())
    }

    fn agent(&self) -> reqwest::Client {
        let builder = reqwest::ClientBuilder::new().timeout(Duration::from_secs(10));
        builder.build().unwrap()
    }

    async fn create_intake(
        &self,
        game_label: String,
        language: Language,
        start_time: u64,
        end_time: Option<u64>,
    ) -> (String, u64) {
        #[derive(Debug, Serialize, Deserialize)]
        struct Request {
            #[serde(rename = "startTime")]
            start_time: u64,
            #[serde(rename = "endTime")]
            end_time: Option<u64>,
            #[serde(rename = "game")]
            game_label: String,
            language: String,
        }

        let url = &self.intake_url;

        let request = Request {
            start_time,
            end_time,
            game_label,
            language: language.intake_str(),
        };

        loop {
            let submitted = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            match self.agent().post(url).json(&request).send().await {
                Ok(res) => match res.json().await {
                    Ok(IntakeResponse {
                        error: Some(error), ..
                    }) => {
                        error!("Error POSTing {url:?} from server: {error}")
                    }
                    Ok(IntakeResponse {
                        message: Some(message),
                        object: Some(IntakeResponseObject { rowid }),
                        ..
                    }) => {
                        info!("Success POSTing {url:?}: {message}");
                        break (rowid, submitted);
                    }
                    Ok(res) => {
                        error!("Error pattern-matching POST {url:?} response: {res:?}")
                    }
                    Err(e) => {
                        error!("Error decoding POST {url:?} response as JSON: {e:?}")
                    }
                },
                Err(e) => {
                    error!("Error POSTing {url:?} with {request:?}: {e:?}");
                }
            }

            info!("Sleeping for 5s before trying again");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }

    async fn finish_intake(&self, intake_id: String, end_time: u64) -> u64 {
        #[derive(Debug, Serialize, Deserialize)]
        struct Request {
            rowid: String,
            #[serde(rename = "endTime")]
            end_time: u64,
        }

        let url = &self.intake_url;

        let request = Request {
            rowid: intake_id,
            end_time,
        };

        loop {
            let submitted = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            match self.agent().patch(url).json(&request).send().await {
                Ok(res) => match res.json().await {
                    Ok(IntakeResponse {
                        error: Some(error), ..
                    }) => {
                        error!("Error PATCHing {url:?} from server: {error}")
                    }
                    Ok(IntakeResponse {
                        message: Some(message),
                        ..
                    }) => {
                        info!("Success PATCHing {url:?}: {message}");
                        break submitted;
                    }
                    Ok(res) => {
                        error!("Error pattern-matching PATCH {url:?} response: {res:?}")
                    }
                    Err(e) => {
                        error!("Error decoding PATCH {url:?} response as JSON: {e:?}")
                    }
                },
                Err(e) => {
                    error!("Error PATCHing {url:?} with {request:?}: {e:?}");
                }
            }

            info!("Sleeping for 5s before trying again");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }
}

impl Language {
    fn intake_str(&self) -> String {
        let lang = match self {
            Language::English => "English",
            Language::Japanese => "日本語",
            Language::Cantonese => "廣東話",
            Language::Other(lang) => {
                warn!("Mapping intake language {lang} to English");
                "English"
            }
        };
        lang.to_string()
    }
}
