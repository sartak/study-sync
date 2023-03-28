use crate::{database::Database, intake, screenshots};
use anyhow::{anyhow, Result};
use log::{error, info};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs::{create_dir_all, hard_link, remove_file, rename};
use tokio::join;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum Language {
    English,
    Japanese,
    Cantonese,
    Other(String),
}

#[derive(Debug)]
pub struct Game {
    pub id: i64,
    pub path: PathBuf,
    pub directory: String,
    pub language: Language,
    pub label: String,
}

#[derive(Debug)]
pub struct Play {
    pub id: i64,
    pub game: Game,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub intake_id: Option<String>,
    pub submitted_start: Option<u64>,
    pub submitted_end: Option<u64>,
    pub skipped: bool,
}

#[derive(Debug)]
pub enum Event {
    GameStarted(PathBuf),
    GameEnded(PathBuf),
    ScreenshotCreated(PathBuf),
    SaveFileCreated(PathBuf),
    IntakeStarted {
        play_id: i64,
        intake_id: String,
        submitted_start: u64,
    },
    IntakeEnded {
        play_id: i64,
        submitted_end: u64,
    },
    IntakeFull {
        play_id: i64,
        intake_id: String,
        submitted_start: u64,
        submitted_end: u64,
    },
    StartShutdown,
}

pub struct OrchestratorPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Orchestrator {
    rx: mpsc::UnboundedReceiver<Event>,
    intake_tx: mpsc::UnboundedSender<intake::Event>,
    screenshots_tx: mpsc::UnboundedSender<screenshots::Event>,
    hold_screenshots: PathBuf,
    trim_game_prefix: Option<String>,
    database: Database,
    current_play: Option<Play>,
    previous_play: Option<Play>,
}

pub fn launch() -> (OrchestratorPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (OrchestratorPre { rx }, tx)
}

impl OrchestratorPre {
    pub async fn start(
        self,
        database: Database,
        hold_screenshots: PathBuf,
        trim_game_prefix: Option<String>,
        intake_tx: mpsc::UnboundedSender<intake::Event>,
        screenshots_tx: mpsc::UnboundedSender<screenshots::Event>,
    ) -> Result<()> {
        if !hold_screenshots.is_dir() {
            return Err(anyhow!(
                "hold-screenshots {hold_screenshots:?} not a directory"
            ));
        }

        for entry in walkdir::WalkDir::new(&hold_screenshots)
            .sort_by_file_name()
            .min_depth(3)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.into_path();
            let mut directory = path.clone();
            directory.pop();
            let directory = directory.strip_prefix(&hold_screenshots)?;
            info!("Found batched screenshot {path:?} for {directory:?}");
            if let Some(directory) = directory.to_str() {
                let event = screenshots::Event::UploadScreenshot(path, directory.to_owned());
                screenshots_tx.send(event)?;
            }
        }

        let (previous, backlog) = join!(
            database.load_previously_playing(),
            database.load_intake_backlog(),
        );

        let previous = previous?;
        match &previous {
            Some(p) => {
                info!("Found previously-playing game {p:?}");
                if p.end_time.is_none() {
                    if let Some(intake_id) = &p.intake_id {
                        intake_tx.send(intake::Event::PreviousGame {
                            play_id: p.id,
                            intake_id: intake_id.clone(),
                        })?;
                    }
                }
            }
            None => info!("No previously-playing game found"),
        };

        let backlog = backlog?;
        if backlog.is_empty() {
            info!("No backlog of intake submissions found");
        } else {
            info!(
                "Found backlog of {} intake submissions: {backlog:?}",
                backlog.len()
            );
            for e in backlog {
                intake_tx.send(e)?;
            }
        }

        let orchestrator = Orchestrator {
            rx: self.rx,
            intake_tx,
            screenshots_tx,
            hold_screenshots,
            trim_game_prefix,
            database,
            current_play: previous,
            previous_play: None,
        };
        orchestrator.start().await
    }
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

                    let (remove_res, game_res) = join!(
                        remove_file(&latest_screenshot),
                        self.database.game_for_path(&path),
                    );

                    if let Err(e) = remove_res {
                        if e.kind() != std::io::ErrorKind::NotFound {
                            error!(
                                "Could not remove latest screenshot {latest_screenshot:?}: {e:?}"
                            );
                            continue;
                        }
                    }

                    let game = match game_res {
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

                    if let Some(play) = &self.current_play {
                        let game = &play.game;
                        let event = intake::Event::SubmitStarted {
                            play_id: play.id,
                            game_label: game.label.clone(),
                            language: game.language.clone(),
                            start_time: play.start_time,
                        };
                        if let Err(e) = self.intake_tx.send(event) {
                            error!("Could not send to intake: {e:?}");
                        }
                    }

                    let dir = self.current_dir().unwrap();
                    if let Err(e) = create_dir_all(&dir).await {
                        error!("Could not create {dir:?}: {e:?}");
                    }
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
                            if let Some(play) = &self.current_play {
                                let game = &play.game;
                                let event = intake::Event::SubmitFull {
                                    play_id: play.id,
                                    game_label: game.label.clone(),
                                    language: game.language.clone(),
                                    start_time: play.start_time,
                                    end_time: play.end_time.unwrap(),
                                };
                                if let Err(e) = self.intake_tx.send(event) {
                                    error!("Could not send to intake: {e:?}");
                                }
                            }
                        }
                    } else {
                        error!("No previous game!");
                    }

                    self.set_current_play(None);
                }

                Event::ScreenshotCreated(path) => {
                    if let Some(play) = self.playing() {
                        let mut destination = self.current_dir().unwrap();
                        destination.push(self.now());
                        destination.set_extension(
                            path.extension()
                                .unwrap_or_else(|| std::ffi::OsStr::new("png")),
                        );

                        info!("Moving screenshot {path:?} to {destination:?} for {play:?}");

                        let (rename_res, remove_res) =
                            join!(rename(&path, &destination), remove_file(&latest_screenshot));

                        if let Err(e) = rename_res {
                            error!("Could not move screenshot {path:?} to {destination:?}: {e:?}");
                            continue;
                        }

                        if let Err(e) = remove_res {
                            if e.kind() != std::io::ErrorKind::NotFound {
                                error!(
                                "Could not remove latest screenshot {latest_screenshot:?}: {e:?}"
                            );
                                continue;
                            }
                        }

                        if let Err(e) = hard_link(&destination, &latest_screenshot).await {
                            error!("Could not hardlink screenshot {path:?} to {latest_screenshot:?}: {e:?}");
                            continue;
                        }

                        let event = screenshots::Event::UploadScreenshot(
                            destination,
                            play.game.directory.clone(),
                        );
                        if let Err(e) = self.screenshots_tx.send(event) {
                            error!("Could not send to screenshots: {e:?}");
                        }
                    } else {
                        let mut destination = extra_directory.clone();
                        destination.push(path.file_name().unwrap());
                        error!("Dropping screenshot {path:?} to {destination:?} because no current playing!");
                        if let Err(e) = rename(&path, &destination).await {
                            error!("Could not move screenshot {path:?} to {destination:?}: {e:?}");
                            continue;
                        }
                    }
                }

                Event::SaveFileCreated(path) => {}

                Event::IntakeStarted {
                    play_id,
                    intake_id,
                    submitted_start,
                } => {
                    if let Some(play) = &mut self.current_play {
                        if play.id == play_id {
                            play.intake_id = Some(intake_id.clone());
                            play.submitted_start = Some(submitted_start);
                        }
                    }

                    if let Err(e) = self
                        .database
                        .initial_intake(play_id, intake_id, submitted_start)
                        .await
                    {
                        error!("Could not update intake: {e:?}")
                    }
                }

                Event::IntakeEnded {
                    play_id,
                    submitted_end,
                } => {
                    if let Some(play) = &mut self.current_play {
                        if play.id == play_id {
                            play.submitted_end = Some(submitted_end);
                        }
                    }

                    if let Err(e) = self.database.final_intake(play_id, submitted_end).await {
                        error!("Could not update intake: {e:?}")
                    }
                }

                Event::IntakeFull {
                    play_id,
                    intake_id,
                    submitted_start,
                    submitted_end,
                } => {
                    if let Some(play) = &mut self.current_play {
                        if play.id == play_id {
                            play.intake_id = Some(intake_id.clone());
                            play.submitted_start = Some(submitted_start);
                            play.submitted_end = Some(submitted_end);
                        }
                    }

                    if let Err(e) = self
                        .database
                        .full_intake(play_id, intake_id, submitted_start, submitted_end)
                        .await
                    {
                        error!("Could not update intake: {e:?}")
                    }
                }

                Event::StartShutdown => {}
            }
        }
        Ok(())
    }

    fn playing(&self) -> Option<&Play> {
        self.current_play.as_ref().or(self.previous_play.as_ref())
    }

    fn current_dir(&self) -> Option<PathBuf> {
        self.current_play
            .as_ref()
            .map(|p| self.hold_screenshots.join(&p.game.directory))
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

    fn now(&self) -> String {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .to_string()
    }
}
