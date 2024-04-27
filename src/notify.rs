use crate::internal::channel::{Action, PriorityRetryChannel};
use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing::{error, info};

#[derive(Debug)]
pub enum Event {
    Success(bool, String),
    Error(String),
    Emergency(String),
    StartShutdown,
}

pub struct NotifyPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Notify {
    led_path: PathBuf,
}

pub fn prepare() -> (NotifyPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (NotifyPre { rx }, tx)
}

impl NotifyPre {
    pub async fn start(self, led_path: PathBuf) -> Result<()> {
        let notify = Notify { led_path };
        notify.start(self.rx).await
    }
}

pub async fn blink_success(quick: bool, led_path: &Path) {
    blink_red(led_path).await;
    wait(if quick { 100 } else { 500 }).await;
    blink_green(led_path).await;

    wait(500).await;
}

pub async fn blink_error(led_path: &Path) {
    for _ in 1..3 {
        blink_red(led_path).await;
        wait(250).await;
        blink_green(led_path).await;
        wait(250).await;
    }

    wait(250).await;
}

pub async fn blink_emergency(led_path: &Path) {
    for _ in 1..10 {
        blink_red(led_path).await;
        wait(100).await;
        blink_green(led_path).await;
        wait(100).await;
    }

    wait(900).await;
}

async fn wait(ms: u64) {
    tokio::time::sleep(tokio::time::Duration::from_millis(ms)).await;
}

async fn blink_red(led_path: &Path) {
    if let Err(e) = change_led(led_path, true).await {
        error!("{e:?}");
    }
}

async fn blink_green(led_path: &Path) {
    if let Err(e) = change_led(led_path, false).await {
        error!("{e:?}");
    }
}

async fn change_led(led_path: &Path, red: bool) -> Result<()> {
    let mut file = File::create(led_path).await?;
    let bytes = if red { b"1" } else { b"0" };
    file.write_all(bytes).await?;
    Ok(())
}

impl Notify {
    pub async fn start(mut self, rx: mpsc::UnboundedReceiver<Event>) -> Result<()> {
        self.run(rx).await;
        info!("notify gracefully shut down");
        Ok(())
    }
}

#[async_trait]
impl PriorityRetryChannel for Notify {
    type Event = Event;

    // Doesn't really matter
    fn is_online(&self) -> bool {
        true
    }

    fn is_high_priority(&self, event: &Event) -> bool {
        match event {
            Event::StartShutdown => true,

            Event::Success(_, _) => false,
            Event::Error(_) => false,
            Event::Emergency(_) => false,
        }
    }

    async fn handle(&mut self, event: &Event) -> Action {
        info!("Handling event {event:?}");

        match event {
            Event::StartShutdown => Action::Halt,

            Event::Success(quick, _) => {
                blink_success(*quick, &self.led_path).await;
                Action::Continue
            }

            Event::Error(_) => {
                blink_error(&self.led_path).await;
                Action::Continue
            }

            Event::Emergency(_) => {
                blink_emergency(&self.led_path).await;
                Action::Continue
            }
        }
    }
}
