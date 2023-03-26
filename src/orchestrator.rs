use crate::game::Play;
use crate::{database::Database, intake};
use anyhow::{anyhow, Result};
use log::{error, info};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Event {
    GameStarted(PathBuf),
    GameEnded(PathBuf),
    ScreenshotCreated(PathBuf),
    SaveFileCreated(PathBuf),
    StartShutdown,
}

pub struct Orchestrator {
    rx: mpsc::UnboundedReceiver<Event>,
    intake_tx: mpsc::UnboundedSender<intake::Event>,
    hold_screenshots: PathBuf,
    trim_game_prefix: Option<String>,
    database: Database,
    current_play: Option<Play>,
    previous_play: Option<Play>,
}

pub async fn launch(
    database: Database,
    hold_screenshots: PathBuf,
    trim_game_prefix: Option<String>,
    intake_tx: mpsc::UnboundedSender<intake::Event>,
) -> Result<(Orchestrator, mpsc::UnboundedSender<Event>)> {
    let (tx, rx) = mpsc::unbounded_channel();

    if !hold_screenshots.is_dir() {
        return Err(anyhow!(
            "hold-screenshots {hold_screenshots:?} not a directory"
        ));
    }

    let previous = database.load_previously_playing().await?;
    match &previous {
        Some(p) => info!("Found previously-playing game {p:?}"),
        None => info!("No previously-playing game found"),
    };

    return Ok((
        Orchestrator {
            rx,
            intake_tx,
            hold_screenshots,
            trim_game_prefix,
            database,
            current_play: previous,
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

                    let play = match self.database.started_playing(game).await {
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

                    if let Some(play) = self.current_play.take() {
                        if play.game.path != path {
                            error!("Previous game does not match! {path:?}, expected {play:?}");
                        } else {
                            self.current_play = Some(self.database.finished_playing(play).await?);
                            info!("Play ended {:?}", self.current_play);
                        }
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

        self.database
            .detach_save_currently_playing(self.current_play.as_ref().map(|p| p.id))
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
