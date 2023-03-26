use crate::game::Language;
use anyhow::Result;
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
}

pub fn launch() -> (IntakePre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    return (IntakePre { rx }, tx);
}

impl IntakePre {
    pub async fn start(self) -> Result<()> {
        let intake = Intake { rx: self.rx };
        intake.start().await
    }
}

impl Intake {
    pub async fn start(mut self) -> Result<()> {
        while let Some(event) = self.rx.recv().await {
            match event {
                Event::SubmitStarted {
                    play_id,
                    game_label,
                    language,
                    start_time,
                } => {}
                Event::SubmitEnded {
                    play_id,
                    intake_id,
                    end_time,
                } => {}
                Event::SubmitFull {
                    play_id,
                    game_label,
                    language,
                    start_time,
                    end_time,
                } => {}
            }
        }
        Ok(())
    }
}
