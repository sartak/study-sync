use crate::event::Event;
use anyhow::Result;
use log::info;
use tokio::sync::mpsc;

pub struct Orchestrator {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub fn launch() -> (Orchestrator, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    return (Orchestrator { rx }, tx);
}

impl Orchestrator {
    pub async fn start(self) -> Result<()> {
        let mut rx = self.rx;
        while let Some(event) = rx.recv().await {
            info!("Handling {event:?}")
        }
        Ok(())
    }
}
