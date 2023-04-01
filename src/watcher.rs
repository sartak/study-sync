use crate::{
    internal::fs,
    notify::{self, Notifier},
    orchestrator,
};
use anyhow::{anyhow, Result};
use log::info;
use regex::Regex;
use std::path::{Path, PathBuf};
use tokio::{select, sync::mpsc};

#[derive(Debug)]
pub enum Event {
    StartShutdown,
}

#[derive(PartialEq)]
pub enum WatchTarget {
    Screenshots,
    SaveFiles,
}

pub struct WatcherPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Watcher {
    rx: mpsc::UnboundedReceiver<Event>,
    fs_rx: mpsc::UnboundedReceiver<PathBuf>,
    notify_tx: mpsc::UnboundedSender<notify::Event>,
    target: WatchTarget,
    orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
}

pub fn prepare() -> (WatcherPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (WatcherPre { rx }, tx)
}

impl WatcherPre {
    pub async fn start(
        self,
        paths: &[PathBuf],
        target: WatchTarget,
        orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
        notify_tx: mpsc::UnboundedSender<notify::Event>,
    ) -> Result<()> {
        let (fs_tx, fs_rx) = mpsc::unbounded_channel();

        {
            let paths = paths.to_owned();
            tokio::spawn(async move { fs::launch(&paths, fs_tx).await });
        }

        let watcher = Watcher {
            rx: self.rx,
            fs_rx,
            target,
            orchestrator_tx,
            notify_tx,
        };

        if watcher.target == WatchTarget::Screenshots {
            for dir in paths {
                watcher.check_directory(dir);
            }
        }

        watcher.start().await
    }
}

impl Watcher {
    pub fn check_directory(&self, directory: &Path) {
        for entry in walkdir::WalkDir::new(directory)
            .sort_by_file_name()
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            self.maybe_emit(entry.into_path());
        }
    }

    pub async fn start(mut self) -> Result<()> {
        loop {
            select! {
                msg = self.rx.recv() => {
                    if let Some(event) = msg {
                        match event {
                            Event::StartShutdown => break,
                        }
                    }
                },
                msg = self.fs_rx.recv() => {
                    match msg {
                        Some(path) => {
                            self.maybe_emit(path);
                        },
                        None => {
                            return Err(anyhow!("filesystem watcher channel unexpectedly closed"));
                        }
                    }
                },
            }
        }

        match self.target {
            WatchTarget::Screenshots => info!("screenshot watcher gracefully shut down"),
            WatchTarget::SaveFiles => info!("save watcher gracefully shut down"),
        };

        Ok(())
    }

    fn maybe_emit(&self, path: PathBuf) {
        let pattern = self.target.file_pattern();

        match path.to_str() {
            Some(p) if pattern.is_match(p) => {}
            _ => return,
        };
        info!("Handling path {path:?}");

        let event = match self.target {
            WatchTarget::Screenshots => orchestrator::Event::ScreenshotCreated(path),
            WatchTarget::SaveFiles => orchestrator::Event::SaveFileCreated(path),
        };
        if let Err(e) = self.orchestrator_tx.send(event) {
            self.notify_error(&format!("Failed to send to orchestrator: {e:?}"));
        }
    }
}

impl WatchTarget {
    fn file_pattern(&self) -> Regex {
        match self {
            WatchTarget::Screenshots => Regex::new(r"\.(?:png|jpg)$").unwrap(),
            WatchTarget::SaveFiles => {
                Regex::new(r"\.(?:srm|state[0-9]*|state\.auto|sav|rtc|ldci)$").unwrap()
            }
        }
    }
}

impl Notifier for Watcher {
    fn notify_target(&self) -> &str {
        "study_sync::watcher"
    }

    fn notify_tx(&self) -> &mpsc::UnboundedSender<notify::Event> {
        &self.notify_tx
    }
}
