use anyhow::Result;
use log::info;
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Event {
    UploadScreenshot(PathBuf, String),
}

pub struct ScreenshotsPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Screenshots {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub fn launch() -> (ScreenshotsPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    return (ScreenshotsPre { rx }, tx);
}

impl ScreenshotsPre {
    pub async fn start(self) -> Result<()> {
        let screenshots = Screenshots { rx: self.rx };
        screenshots.start().await
    }
}

impl Screenshots {
    pub async fn start(mut self) -> Result<()> {
        while let Some(event) = self.rx.recv().await {
            info!("Handling event {event:?}");
            match event {
                Event::UploadScreenshot(path, directory) => {}
            }
        }
        Ok(())
    }
}
