use crate::game::Language;
use anyhow::Result;
use log::info;
use std::collections::HashMap;
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
        intake_id: u64,
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

pub struct IntakePre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Intake {
    rx: mpsc::UnboundedReceiver<Event>,
    play_to_intake: HashMap<i64, u64>,
}

pub fn launch() -> (IntakePre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    return (IntakePre { rx }, tx);
}

impl IntakePre {
    pub async fn start(self) -> Result<()> {
        let intake = Intake {
            rx: self.rx,
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
                    self.play_to_intake.insert(play_id, intake_id);
                }
                Event::SubmitEnded {
                    play_id,
                    intake_id,
                    end_time,
                } => {
                    let submitted_end = self.finish_intake(intake_id, end_time).await;
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
                    } else {
                        let (intake_id, submitted_start) = self
                            .create_intake(game_label, language, start_time, Some(end_time))
                            .await;
                    }
                }
            }
        }
        Ok(())
    }

    async fn create_intake(
        &self,
        game_label: String,
        language: Language,
        start_time: u64,
        end_time: Option<u64>,
    ) -> (u64, u64) {
        let intake_id = 0;
        let submitted = 0;

        (intake_id, submitted)
    }

    async fn finish_intake(&self, intake_id: u64, end_time: u64) -> u64 {
        let submitted = 0;

        submitted
    }
}
