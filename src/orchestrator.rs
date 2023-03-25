use crate::database::Database;
use crate::event::Event;
use crate::game::Play;
use anyhow::{anyhow, Result};
use log::{error, info};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub struct Orchestrator {
    rx: mpsc::UnboundedReceiver<Event>,
    hold_screenshots: PathBuf,
    trim_game_prefix: Option<String>,
    database: Database,
    current_play: Option<Play>,
    previous_play: Option<Play>,
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
            current_play: None,
            previous_play: None,
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
                    if let Some(previous_play) = &self.current_play {
                        error!("Already have a current play! {previous_play:?}");
                    }

                    let path = match self.fixed_path(&path) {
                        Some(p) => p,
                        None => continue,
                    };
                    let game = match self.database.game_for_path(&path).await {
                        Ok(game) => game,
                        Err(e) => {
                            error!("Could not find game for path {path:?}: {e:?}");
                            continue;
                        }
                    };

                    let play = match self.database.start_playing(game).await {
                        Ok(play) => play,
                        Err(e) => {
                            error!("Could not start play: {e:?}");
                            continue;
                        }
                    };

                    info!("Play begin {play:?}");
                    self.set_current_play(Some(play));
                }
                Event::GameEnded(path) => {
                    let path = match self.fixed_path(&path) {
                        Some(p) => p,
                        None => continue,
                    };

                    if let Some(previous_play) = &self.current_play {
                        if previous_play.game.path != path {
                            error!(
                                "Previous game does not match! {path:?}, expected {previous_play:?}"
                            );
                        }
                        info!("Play ended {previous_play:?}");
                    } else {
                        error!("No previous game!");
                    }

                    self.set_current_play(None);
                }
                Event::ScreenshotCreated(path) => {
                    let play = match self.playing() {
                        Some(p) => p,
                        None => {
                            error!("Screenshot {path:?} created but no current playing!");
                            todo!("move screenshot into {extra_directory:?}");
                        }
                    };

                    info!("Got screenshot {path:?} for {play:?}");
                }
                Event::SaveFileCreated(path) => {}
                Event::StartShutdown => {}
            }
        }
        Ok(())
    }

    fn playing(&self) -> Option<&Play> {
        self.current_play.as_ref().or(self.previous_play.as_ref())
    }

    fn set_current_play(&mut self, play: Option<Play>) {
        let current = self.current_play.take();
        if current.is_some() {
            self.previous_play = current;
        }
        self.current_play = play;
    }

    fn fixed_path<'p>(&self, path: &'p Path) -> Option<&'p Path> {
        match self.trim_game_prefix {
            Some(ref prefix) => match path.strip_prefix(&prefix) {
                Ok(p) => Some(p),
                Err(e) => {
                    error!("Could not trim prefix {prefix:?} from {path:?}: {e:?}");
                    None
                }
            },
            None => Some(&path),
        }
    }
}
