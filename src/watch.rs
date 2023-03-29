mod fs;

use crate::{
    notify::{self, Notifier},
    orchestrator,
};
use anyhow::{anyhow, Result};
use log::info;
use std::path::PathBuf;
use tokio::{select, sync::mpsc};

#[derive(Debug)]
pub enum Event {
    StartShutdown,
}

pub enum WatchTarget {
    Screenshots,
    SaveFiles,
}

pub struct WatchPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Watch {
    rx: mpsc::UnboundedReceiver<Event>,
    fs_rx: mpsc::UnboundedReceiver<PathBuf>,
    notify_tx: mpsc::UnboundedSender<notify::Event>,
    target: WatchTarget,
    orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
}

pub fn prepare() -> (WatchPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (WatchPre { rx }, tx)
}

impl WatchPre {
    pub async fn start(
        self,
        paths: &[PathBuf],
        target: WatchTarget,
        orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
        notify_tx: mpsc::UnboundedSender<notify::Event>,
    ) -> Result<()> {
        let (fs_tx, fs_rx) = mpsc::unbounded_channel();

        let paths = paths.to_owned();
        tokio::spawn(async move { fs::launch(&paths, fs_tx).await });

        let watch = Watch {
            rx: self.rx,
            fs_rx,
            target,
            orchestrator_tx,
            notify_tx,
        };
        watch.start().await
    }
}

impl Watch {
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
                            info!("Handling path {path:?}");
                            let event = match self.target {
                                WatchTarget::Screenshots => orchestrator::Event::ScreenshotCreated(path),
                                WatchTarget::SaveFiles => orchestrator::Event::SaveFileCreated(path),
                            };
                            if let Err(e) = self.orchestrator_tx.send(event) {
                                self.notify_error(&format!("Failed to send to orchestrator: {e:?}"));
                            }
                        },
                        None => {
                            return Err(anyhow!("filesystem watcher channel unexpectedly closed"));
                        }
                    }
                },
            }
        }

        info!("watch gracefully shut down");
        Ok(())
    }
}

impl Notifier for Watch {
    fn notify_target(&self) -> &str {
        "study_sync::watch"
    }

    fn notify_tx(&self) -> &mpsc::UnboundedSender<notify::Event> {
        &self.notify_tx
    }
}
