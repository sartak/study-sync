use anyhow::Result;
use log::{error, info};
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

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
    rx: mpsc::UnboundedReceiver<Event>,
    led_path: PathBuf,
}

pub fn prepare() -> (NotifyPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (NotifyPre { rx }, tx)
}

impl NotifyPre {
    pub async fn start(self, led_path: PathBuf) -> Result<()> {
        let notify = Notify {
            rx: self.rx,
            led_path,
        };
        notify.start().await
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
    pub async fn start(mut self) -> Result<()> {
        while let Some(event) = self.rx.recv().await {
            info!("Handling {event:?}");
            match event {
                Event::Success(quick, _) => blink_success(quick, &self.led_path).await,
                Event::Error(_) => blink_error(&self.led_path).await,
                Event::Emergency(_) => blink_emergency(&self.led_path).await,
                Event::StartShutdown => break,
            }
        }

        info!("notify gracefully shut down");
        Ok(())
    }
}

pub trait Notifier {
    fn notify_target(&self) -> &str;

    fn notify_tx(&self) -> &mpsc::UnboundedSender<Event>;

    fn notify_success(&self, quick: bool, message: &str) {
        info!(target: self.notify_target(), "Success: {message:?}");

        if let Err(e) = self
            .notify_tx()
            .send(Event::Success(quick, message.to_owned()))
        {
            error!(target: self.notify_target(), "Could not send success {message:?} to notify: {e:?}");
        }
    }

    fn notify_error(&self, message: &str) {
        error!(target: self.notify_target(), "Error: {message:?}");

        if let Err(e) = self.notify_tx().send(Event::Error(message.to_owned())) {
            error!(target: self.notify_target(), "Could not send error {message:?} to notify: {e:?}");
        }
    }

    fn notify_emergency(&self, message: &str) {
        error!(target: self.notify_target(), "Emergency: {message:?}");

        if let Err(e) = self.notify_tx().send(Event::Emergency(message.to_owned())) {
            error!(target: self.notify_target(), "Could not send emergency {message:?} to notify: {e:?}");
        }
    }
}
