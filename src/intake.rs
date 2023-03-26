use crate::game::Play;
use anyhow::{anyhow, Result};
use log::{error, info};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub enum Event {}

pub struct Intake {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub async fn launch() -> Result<(Intake, mpsc::UnboundedSender<Event>)> {
    let (tx, rx) = mpsc::unbounded_channel();

    return Ok((Intake { rx }, tx));
}

impl Intake {
    pub async fn start(mut self) -> Result<()> {
        while let Some(event) = self.rx.recv().await {
            match event {}
        }
        Ok(())
    }
}
