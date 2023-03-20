use crate::event::Event;
use crate::game::Game;
use anyhow::Result;
use log::{error, info};
use tokio::sync::mpsc;

pub struct Orchestrator {
    rx: mpsc::UnboundedReceiver<Event>,
    current_game: Option<Game>,
    previous_game: Option<Game>,
}

pub fn launch() -> (Orchestrator, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    return (
        Orchestrator {
            rx,
            current_game: None,
            previous_game: None,
        },
        tx,
    );
}

impl Orchestrator {
    pub async fn start(mut self) -> Result<()> {
        while let Some(event) = self.rx.recv().await {
            info!("Handling {event:?}");
            match event {
                Event::GameStarted(path) => {
                    if let Some(previous_game) = &self.current_game {
                        error!("Already have a current game! {previous_game:?}");
                    }

                    self.set_current_game(Some(Game { path }));
                }
                Event::GameEnded(path) => {
                    if let Some(previous_game) = &self.current_game {
                        if previous_game.path != path {
                            error!("Previous game does not match! {previous_game:?}");
                        }
                    } else {
                        error!("No previous game!");
                    }

                    self.set_current_game(None);
                }
                Event::ScreenshotCreated(path) => {}
                Event::SaveFileCreated(path) => {}
                Event::StartShutdown => {}
            }
        }
        Ok(())
    }

    fn game(&self) -> Option<&Game> {
        self.current_game.as_ref().or(self.previous_game.as_ref())
    }

    fn set_current_game(&mut self, game: Option<Game>) {
        let current = self.current_game.take();
        if current.is_some() {
            self.previous_game = current;
        }
        self.current_game = game;
    }
}
