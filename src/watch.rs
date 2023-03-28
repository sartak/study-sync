mod fs;

use crate::orchestrator;
use anyhow::Result;
use log::{error, info};
use std::path::PathBuf;
use tokio::sync::mpsc;

pub enum WatchTarget {
    Screenshots,
    SaveFiles,
}

pub struct WatchPre {}

pub struct Watch {
    fs_rx: mpsc::UnboundedReceiver<PathBuf>,
    target: WatchTarget,
    orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
}

pub fn launch() -> WatchPre {
    WatchPre {}
}

impl WatchPre {
    pub async fn start(
        self,
        paths: Vec<PathBuf>,
        target: WatchTarget,
        orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
    ) -> Result<()> {
        let (fs_tx, fs_rx) = mpsc::unbounded_channel();

        tokio::spawn(async move { fs::launch(paths, fs_tx).await });

        let watch = Watch {
            fs_rx,
            target,
            orchestrator_tx,
        };
        watch.start().await
    }
}

impl Watch {
    pub async fn start(mut self) -> Result<()> {
        while let Some(path) = self.fs_rx.recv().await {
            info!("Handling path {path:?}");
            let event = match self.target {
                WatchTarget::Screenshots => orchestrator::Event::ScreenshotCreated(path),
                WatchTarget::SaveFiles => orchestrator::Event::SaveFileCreated(path),
            };
            if let Err(e) = self.orchestrator_tx.send(event) {
                error!("Failed to send to orchestrator: {e:?}");
            }
            /*
            match event {
                Event::StartShutdown => return Ok(()),
            }
            */
        }
        Ok(())
    }
}
