use crate::game::Play;
use anyhow::{anyhow, Result};
use log::{error, info};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;


pub struct IntakePre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Intake {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub fn launch() -> (IntakePre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    return (IntakePre { rx }, tx);
}

impl IntakePre {
    pub async fn start(self) -> Result<()> {
        let intake = Intake { rx: self.rx };
        intake.start().await
    }
}

impl Intake {
    pub async fn start(mut self) -> Result<()> {
        while let Some(event) = self.rx.recv().await {
            match event {}
        }
        Ok(())
    }
}
