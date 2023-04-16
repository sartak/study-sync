use crate::{
    database::Database,
    intake,
    internal::{
        fs::{full_extension, now_milli, now_ymd, recursive_files_in, remove_full_extension},
        notifier::Notifier,
    },
    notify, saves, screenshots, server, watcher,
};
use anyhow::Result;
use log::{error, info};
use std::path::{Path, PathBuf};
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
    IsOnline(bool),
    ForceSync,
    StartShutdown,
}

pub struct OrchestratorPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Orchestrator {
    rx: mpsc::UnboundedReceiver<Event>,
    intake_tx: mpsc::UnboundedSender<intake::Event>,
    screenshots_tx: mpsc::UnboundedSender<screenshots::Event>,
    saves_tx: mpsc::UnboundedSender<saves::Event>,
    screenshot_watcher_tx: mpsc::UnboundedSender<watcher::Event>,
    save_watcher_tx: mpsc::UnboundedSender<watcher::Event>,
    server_tx: mpsc::UnboundedSender<server::Event>,
    notify_tx: mpsc::UnboundedSender<notify::Event>,
    pending_screenshots: PathBuf,
    pending_saves: PathBuf,
    keep_saves: PathBuf,
    extra_directory: PathBuf,
    latest_screenshot: PathBuf,
    trim_game_prefix: Option<String>,
    database: Database,
    current_play: Option<Play>,
    previous_play: Option<Play>,
}

pub fn prepare() -> (OrchestratorPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (OrchestratorPre { rx }, tx)
}

impl OrchestratorPre {
    #![allow(clippy::too_many_arguments)]
    pub async fn start(
        self,
        database: Database,
        pending_screenshots: PathBuf,
        pending_saves: PathBuf,
        keep_saves: PathBuf,
        extra_directory: PathBuf,
        latest_screenshot: PathBuf,
        trim_game_prefix: Option<String>,
        intake_tx: mpsc::UnboundedSender<intake::Event>,
        screenshots_tx: mpsc::UnboundedSender<screenshots::Event>,
        saves_tx: mpsc::UnboundedSender<saves::Event>,
        screenshot_watcher_tx: mpsc::UnboundedSender<watcher::Event>,
        save_watcher_tx: mpsc::UnboundedSender<watcher::Event>,
        server_tx: mpsc::UnboundedSender<server::Event>,
        notify_tx: mpsc::UnboundedSender<notify::Event>,
    ) -> Result<()> {
        self.upload_existing_screenshots(&pending_screenshots, &screenshots_tx)?;
        self.upload_extra_screenshots(&extra_directory, &screenshots_tx);
        self.upload_existing_saves(&pending_saves, &saves_tx)?;

        let previous = self.load_backlog(&database, &intake_tx).await?;

        let orchestrator = Orchestrator {
            rx: self.rx,
            intake_tx,
            screenshots_tx,
            saves_tx,
            screenshot_watcher_tx,
            save_watcher_tx,
            server_tx,
            notify_tx,
            pending_screenshots,
            pending_saves,
            keep_saves,
            extra_directory,
            latest_screenshot,
            trim_game_prefix,
            database,
            current_play: previous,
            previous_play: None,
        };
        orchestrator.start().await
    }

    fn upload_existing_screenshots(
        &self,
        pending_screenshots: &Path,
        screenshots_tx: &mpsc::UnboundedSender<screenshots::Event>,
    ) -> Result<()> {
        for path in recursive_files_in(pending_screenshots, Some(3)) {
            let mut directory = path.clone();
            directory.pop();
            let directory = directory.strip_prefix(pending_screenshots)?;
            info!("Found batched screenshot {path:?} for {directory:?}");
            if let Some(directory) = directory.to_str() {
                let event = screenshots::Event::UploadScreenshot(path, directory.to_owned());
                screenshots_tx.send(event)?;
            }
        }

        Ok(())
    }

    fn upload_existing_saves(
        &self,
        pending_saves: &Path,
        saves_tx: &mpsc::UnboundedSender<saves::Event>,
    ) -> Result<()> {
        for path in recursive_files_in(pending_saves, None) {
            let mut directory = path.clone();
            directory.pop();
            let directory = directory.strip_prefix(pending_saves)?.to_owned();

            let event = match path.extension().map(|s| s.to_str()) {
                Some(Some("png" | "jpg")) => {
                    info!("Found batched screenshot {path:?} for {directory:?}");
                    saves::Event::UploadScreenshot(path, directory)
                }
                Some(Some(_)) => {
                    info!("Found batched save {path:?} for {directory:?}");
                    saves::Event::UploadSave(path, directory)
                }
                _ => continue,
            };
            saves_tx.send(event)?;
        }

        Ok(())
    }

    fn upload_extra_screenshots(
        &self,
        extra_directory: &Path,
        screenshots_tx: &mpsc::UnboundedSender<screenshots::Event>,
    ) {
        let extra_directory = extra_directory.to_owned();
        let screenshots_tx = screenshots_tx.clone();

        tokio::spawn(async move {
            for path in recursive_files_in(extra_directory, None) {
                info!("Uploading extra screenshot {path:?}");
                let event = screenshots::Event::UploadExtra(path);
                if let Err(e) = screenshots_tx.send(event) {
                    error!("Could not send to screenshots: {e:?}");
                }
            }
        });
    }

    async fn load_backlog(
        &self,
        database: &Database,
        intake_tx: &mpsc::UnboundedSender<intake::Event>,
    ) -> Result<Option<Play>> {
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

        Ok(previous)
    }
}

impl Orchestrator {
    pub async fn start(mut self) -> Result<()> {
        while let Some(event) = self.rx.recv().await {
            info!("Handling {event:?}");
            match event {
                Event::GameStarted(path) => {
                    if let Some(previous_play) = &self.current_play {
                        self.notify_error(&format!(
                            "Already have a current play! {previous_play:?}"
                        ));
                    }

                    let path = match self.trim_game_path(&path) {
                        Some(p) => p,
                        None => continue,
                    };

                    let (remove_res, game_res) = join!(
                        remove_file(&self.latest_screenshot),
                        self.database.game_for_path(path),
                    );

                    if let Err(e) = remove_res {
                        if e.kind() != std::io::ErrorKind::NotFound {
                            self.notify_error(&format!(
                                "Could not remove latest screenshot {:?}: {e:?}",
                                self.latest_screenshot
                            ));
                            continue;
                        }
                    }

                    let game = match game_res {
                        Ok(game) => game,
                        Err(e) => {
                            self.notify_error(&format!(
                                "Could not find game for path {path:?}: {e:?}"
                            ));
                            continue;
                        }
                    };

                    let play = match self.database.started_playing(game).await {
                        Ok(play) => play,
                        Err(e) => {
                            self.notify_error(&format!("Could not start play: {e:?}"));
                            continue;
                        }
                    };

                    self.set_current_play(Some(play));
                    let play = self.current_play.as_ref().unwrap();
                    let game = &play.game;

                    let event = intake::Event::SubmitStarted {
                        play_id: play.id,
                        game_label: game.label.clone(),
                        language: game.language.clone(),
                        start_time: play.start_time,
                    };
                    if let Err(e) = self.intake_tx.send(event) {
                        self.notify_error(&format!("Could not send to intake: {e:?}"));
                    }

                    let screenshot_dir = self.screenshot_dir().unwrap();

                    let mut pending_save_dir = self.pending_saves.join(path);
                    pending_save_dir.set_extension("");

                    let mut keep_save_dir = self.keep_saves.join(path);
                    keep_save_dir.set_extension("");

                    let (screenshot_dir_res, pending_save_dir_res, keep_save_dir_res) = join!(
                        create_dir_all(&screenshot_dir),
                        create_dir_all(&pending_save_dir),
                        create_dir_all(&keep_save_dir)
                    );
                    if let Err(e) = screenshot_dir_res {
                        self.notify_error(&format!("Could not create {screenshot_dir:?}: {e:?}"));
                        continue;
                    }
                    if let Err(e) = pending_save_dir_res {
                        self.notify_error(&format!("Could not create {pending_save_dir:?}: {e:?}"));
                        continue;
                    }
                    if let Err(e) = keep_save_dir_res {
                        self.notify_error(&format!("Could not create {keep_save_dir:?}: {e:?}"));
                        continue;
                    }

                    self.notify_success(false, "Play began!");
                }

                Event::GameEnded(path) => {
                    let path = match self.trim_game_path(&path) {
                        Some(p) => p,
                        None => continue,
                    };

                    if let Some(play) = self.current_play.take() {
                        if play.game.path != path {
                            self.notify_error(&format!(
                                "Previous game does not match! {path:?}, expected {play:?}"
                            ));
                        } else {
                            self.current_play = Some(self.database.finished_playing(play).await?);
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
                                    self.notify_error(&format!("Could not send to intake: {e:?}"));
                                }
                            }
                            self.notify_success(false, "Play ended!");
                        }
                    } else {
                        self.notify_error("No previous game!");
                    }

                    self.set_current_play(None);
                }

                Event::ScreenshotCreated(path) => {
                    if let Some(play) = self.playing() {
                        let mut destination = self.screenshot_dir().unwrap();
                        destination.push(now_milli());
                        destination.set_extension(
                            path.extension()
                                .unwrap_or_else(|| std::ffi::OsStr::new("png")),
                        );

                        info!("Moving screenshot {path:?} to {destination:?} for {play:?}");

                        let (rename_res, remove_res) = join!(
                            rename(&path, &destination),
                            remove_file(&self.latest_screenshot)
                        );

                        if let Err(e) = rename_res {
                            self.notify_error(&format!(
                                "Could not move screenshot {path:?} to {destination:?}: {e:?}"
                            ));
                            continue;
                        }

                        if let Err(e) = remove_res {
                            if e.kind() != std::io::ErrorKind::NotFound {
                                self.notify_error(&format!(
                                    "Could not remove latest screenshot {:?}: {e:?}",
                                    self.latest_screenshot
                                ));
                                continue;
                            }
                        }

                        if let Err(e) = hard_link(&destination, &self.latest_screenshot).await {
                            self.notify_error(&format!(
                                "Could not hardlink screenshot {path:?} to {:?}: {e:?}",
                                self.latest_screenshot
                            ));
                            continue;
                        }

                        let event = screenshots::Event::UploadScreenshot(
                            destination,
                            play.game.directory.clone(),
                        );
                        if let Err(e) = self.screenshots_tx.send(event) {
                            self.notify_error(&format!("Could not send to screenshots: {e:?}"));
                        }
                    } else {
                        let mut destination = self.extra_directory.clone();
                        destination.push(path.file_name().unwrap());
                        self.notify_error(&format!("Dropping screenshot {path:?} to {destination:?} because no current playing!"));
                        if let Err(e) = rename(&path, &destination).await {
                            self.notify_error(&format!(
                                "Could not move screenshot {path:?} to {destination:?}: {e:?}"
                            ));
                            continue;
                        }

                        let event = screenshots::Event::UploadExtra(destination);
                        if let Err(e) = self.screenshots_tx.send(event) {
                            self.notify_error(&format!("Could not send to screenshots: {e:?}"));
                        }
                    }
                }

                Event::SaveFileCreated(path) => {
                    let extension = match full_extension(&path) {
                        Some(e) => e,
                        None => {
                            self.notify_error(&format!(
                                "Could not extract extension from {path:?}"
                            ));
                            continue;
                        }
                    };

                    let mut directory = match self.trim_game_path(&path) {
                        Some(p) => p.to_path_buf(),
                        None => continue,
                    };

                    remove_full_extension(&mut directory);

                    let mut target = directory.join(now_ymd());
                    target.set_extension(extension);

                    let pending_save_destination = self.pending_saves.join(&target);
                    let keep_save_destination = self.keep_saves.join(&target);

                    let mut pending_screenshot_destination = pending_save_destination.clone();
                    let mut keep_screenshot_destination = keep_save_destination.clone();
                    pending_screenshot_destination.set_extension("png");
                    keep_screenshot_destination.set_extension("png");

                    let latest_screenshot = &self.latest_screenshot;

                    let (
                        pending_save_res,
                        keep_save_res,
                        pending_screenshot_res,
                        keep_screenshot_res,
                    ) = join!(
                        hard_link(&path, &pending_save_destination),
                        hard_link(&path, &keep_save_destination),
                        hard_link(&latest_screenshot, &pending_screenshot_destination),
                        hard_link(&latest_screenshot, &keep_screenshot_destination),
                    );

                    if let Err(e) = pending_save_res {
                        self.notify_error(&format!(
                            "Could not hardlink save {path:?} to {pending_save_destination:?}: {e:?}"
                        ));
                        continue;
                    }

                    if let Err(e) = keep_save_res {
                        self.notify_error(&format!(
                            "Could not hardlink save {path:?} to {keep_save_destination:?}: {e:?}"
                        ));
                        continue;
                    }

                    if let Err(e) = keep_screenshot_res {
                        self.notify_error(&format!(
                            "Could not hardlink screenshot {latest_screenshot:?} to {keep_screenshot_destination:?}: {e:?}"
                        ));
                    }

                    if let Err(e) = &pending_screenshot_res {
                        self.notify_error(&format!(
                            "Could not hardlink screenshot {latest_screenshot:?} to {pending_screenshot_destination:?}: {e:?}"
                        ));
                    }

                    let event =
                        saves::Event::UploadSave(pending_save_destination, directory.clone());
                    if let Err(e) = self.saves_tx.send(event) {
                        self.notify_error(&format!("Could not send to saves: {e:?}"));
                        continue;
                    }

                    if pending_screenshot_res.is_ok() {
                        let event = saves::Event::UploadScreenshot(
                            pending_screenshot_destination,
                            directory,
                        );
                        if let Err(e) = self.saves_tx.send(event) {
                            self.notify_error(&format!("Could not send to saves: {e:?}"));
                            continue;
                        }
                    }
                }

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
                        .initial_intake(play_id, &intake_id, submitted_start)
                        .await
                    {
                        self.notify_error(&format!("Could not update intake: {e:?}"));
                        continue;
                    }

                    self.notify_success(true, &format!("Created intake {intake_id:?}"));
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
                        self.notify_error(&format!("Could not update intake: {e:?}"));
                        continue;
                    }

                    self.notify_success(true, "Finished intake");
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
                        .full_intake(play_id, &intake_id, submitted_start, submitted_end)
                        .await
                    {
                        self.notify_error(&format!("Could not update intake: {e:?}"));
                        continue;
                    }

                    self.notify_success(true, &format!("Created full intake {intake_id:?}"));
                }

                Event::IsOnline(online) => {
                    if let Err(e) = self.intake_tx.send(intake::Event::IsOnline(online)) {
                        self.notify_error(&format!("Could not send to intake: {e:?}"));
                    }
                    if let Err(e) = self
                        .screenshots_tx
                        .send(screenshots::Event::IsOnline(online))
                    {
                        self.notify_error(&format!("Could not send to screenshots: {e:?}"));
                    }
                    if let Err(e) = self.saves_tx.send(saves::Event::IsOnline(online)) {
                        self.notify_error(&format!("Could not send to saves: {e:?}"));
                    }
                }

                Event::ForceSync => {
                    if let Err(e) = self.intake_tx.send(intake::Event::ForceSync) {
                        self.notify_error(&format!("Could not send to intake: {e:?}"));
                    }
                    if let Err(e) = self.screenshots_tx.send(screenshots::Event::ForceSync) {
                        self.notify_error(&format!("Could not send to screenshots: {e:?}"));
                    }
                    if let Err(e) = self.saves_tx.send(saves::Event::ForceSync) {
                        self.notify_error(&format!("Could not send to saves: {e:?}"));
                    }
                }

                Event::StartShutdown => {
                    if let Err(e) = self.intake_tx.send(intake::Event::StartShutdown) {
                        self.notify_error(&format!("Could not send to intake: {e:?}"));
                    }
                    if let Err(e) = self.screenshots_tx.send(screenshots::Event::StartShutdown) {
                        self.notify_error(&format!("Could not send to screenshots: {e:?}"));
                    }
                    if let Err(e) = self.saves_tx.send(saves::Event::StartShutdown) {
                        self.notify_error(&format!("Could not send to saves: {e:?}"));
                    }
                    if let Err(e) = self
                        .screenshot_watcher_tx
                        .send(watcher::Event::StartShutdown)
                    {
                        self.notify_error(&format!("Could not send to screenshot_watcher: {e:?}"));
                    }
                    if let Err(e) = self.save_watcher_tx.send(watcher::Event::StartShutdown) {
                        self.notify_error(&format!("Could not send to save_watcher: {e:?}"));
                    }
                    if let Err(e) = self.server_tx.send(server::Event::StartShutdown) {
                        self.notify_error(&format!("Could not send to server: {e:?}"));
                    }
                    if let Err(e) = self.notify_tx.send(notify::Event::StartShutdown) {
                        self.notify_error(&format!("Could not send to notify: {e:?}"));
                    }
                    break;
                }
            }
        }

        info!("orchestrator gracefully shut down");
        Ok(())
    }

    fn playing(&self) -> Option<&Play> {
        self.current_play.as_ref().or(self.previous_play.as_ref())
    }

    fn screenshot_dir(&self) -> Option<PathBuf> {
        self.playing()
            .as_ref()
            .map(|p| self.pending_screenshots.join(&p.game.directory))
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

    fn trim_game_path<'p>(&self, path: &'p Path) -> Option<&'p Path> {
        match self.trim_game_prefix {
            Some(ref prefix) => match path.strip_prefix(prefix) {
                Ok(p) => Some(p),
                Err(e) => {
                    self.notify_error(&format!(
                        "Could not trim prefix {prefix:?} from {path:?}: {e:?}"
                    ));
                    None
                }
            },
            None => Some(path),
        }
    }
}

impl Notifier for Orchestrator {
    fn notify_target(&self) -> &str {
        "study_sync::orchestrator"
    }

    fn notify_tx(&self) -> &mpsc::UnboundedSender<notify::Event> {
        &self.notify_tx
    }
}
