use crate::database::Database;
use crate::event::Event;
use crate::game::Game;
use anyhow::{anyhow, Result};
use log::{error, info};
use std::path::PathBuf;
use tokio::sync::mpsc;

pub struct Orchestrator {
    rx: mpsc::UnboundedReceiver<Event>,
    hold_screenshots: PathBuf,
    trim_game_prefix: Option<String>,
    database: Database,
    current_game: Option<Game>,
    previous_game: Option<Game>,
}

pub fn launch(
    database: Database,
    hold_screenshots: PathBuf,
    trim_game_prefix: Option<String>,
) -> Result<(Orchestrator, mpsc::UnboundedSender<Event>)> {
    let (tx, rx) = mpsc::unbounded_channel();

    if !hold_screenshots.is_dir() {
        return Err(anyhow!(
            "hold-screenshots {hold_screenshots:?} not a directory"
        ));
    }

    return Ok((
        Orchestrator {
            rx,
            hold_screenshots,
            trim_game_prefix,
            database,
            current_game: None,
            previous_game: None,
        },
        tx,
    ));
}

impl Orchestrator {
    pub async fn start(mut self) -> Result<()> {
        let extra_directory = self.hold_screenshots.join("extra/");
        let latest_screenshot = self.hold_screenshots.join("latest.png");

        while let Some(event) = self.rx.recv().await {
            info!("Handling {event:?}");
            match event {
                Event::GameStarted(path) => {
                    if let Some(previous_game) = &self.current_game {
                        error!("Already have a current game! {previous_game:?}");
                    }

                    let path = match self.trim_game_prefix {
                        Some(ref prefix) => match path.strip_prefix(&prefix) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("Could not trim prefix {prefix:?} from {path:?}: {e:?}");
                                continue;
                            }
                        },
                        None => &path,
                    };

                    let game = match self.database.game_for_path(&path).await {
                        Ok(game) => game,
                        Err(e) => {
                            error!("Could not find game for path {path:?}: {e:?}");
                            continue;
                        }
                    };
                    self.set_current_game(Some(game));
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
                Event::ScreenshotCreated(path) => {
                    let game = match self.game() {
                        Some(g) => g,
                        None => {
                            error!("Screenshot {path:?} created but no current game!");
                            todo!("move screenshot into {extra_directory:?}");
                        }
                    };

                    info!("Got screenshot {path:?} for {game:?}");
                }
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
