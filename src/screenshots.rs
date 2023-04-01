use crate::{
    internal::{
        channel::{Action, PriorityRetryChannel},
        notifier::Notifier,
        online::Online,
        uploader::Uploader,
    },
    notify, orchestrator,
};
use anyhow::Result;
use async_trait::async_trait;
use log::{error, info};
use std::path::{Path, PathBuf};
use tokio::fs::remove_file;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Event {
    UploadScreenshot(PathBuf, String),
    UploadExtra(PathBuf),
    IsOnline(bool),
    StartShutdown,
}

pub struct ScreenshotsPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Screenshots {
    orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
    notify_tx: mpsc::UnboundedSender<notify::Event>,
    screenshot_url: String,
    extra_directory: String,
    digest_cache: Option<(PathBuf, String)>,
    is_online: bool,
}

pub fn prepare() -> (ScreenshotsPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (ScreenshotsPre { rx }, tx)
}

impl ScreenshotsPre {
    pub async fn start(
        self,
        orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
        notify_tx: mpsc::UnboundedSender<notify::Event>,
        screenshot_url: String,
        extra_directory: String,
        is_online: bool,
    ) -> Result<()> {
        let mut screenshots = Screenshots {
            orchestrator_tx,
            notify_tx,
            screenshot_url,
            extra_directory,
            digest_cache: None,
            is_online,
        };
        screenshots.start(self.rx).await
    }
}

impl Screenshots {
    async fn start(&mut self, rx: mpsc::UnboundedReceiver<Event>) -> Result<()> {
        self.run(rx).await;
        info!("screenshots gracefully shut down");
        Ok(())
    }

    async fn upload_screenshot(&mut self, path: &Path, directory: &str) -> Result<()> {
        let extension = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("png");

        let content_type = if extension == "jpg" {
            "image/jpeg"
        } else {
            "image/png"
        };

        let url = self.screenshot_url.clone();
        self.upload_path_to_directory(&url, path, directory, Some(content_type))
            .await
    }
}

impl Notifier for Screenshots {
    fn notify_target(&self) -> &str {
        "study_sync::screenshots"
    }

    fn notify_tx(&self) -> &mpsc::UnboundedSender<notify::Event> {
        &self.notify_tx
    }
}

impl Uploader for Screenshots {
    fn get_digest_cache(&self) -> &Option<(PathBuf, String)> {
        &self.digest_cache
    }

    fn set_digest_cache(&mut self, cache: Option<(PathBuf, String)>) {
        self.digest_cache = cache;
    }
}

impl Online for Screenshots {
    fn orchestrator_tx(&self) -> &mpsc::UnboundedSender<orchestrator::Event> {
        &self.orchestrator_tx
    }

    fn is_online(&self) -> bool {
        self.is_online
    }
}

#[async_trait]
impl PriorityRetryChannel for Screenshots {
    type Event = Event;

    fn is_online(&self) -> bool {
        self.is_online
    }

    fn is_high_priority(&self, event: &Event) -> bool {
        match event {
            Event::StartShutdown => true,
            Event::IsOnline(_) => true,

            Event::UploadScreenshot(_, _) => false,
            Event::UploadExtra(_) => false,
        }
    }

    async fn handle(&mut self, event: &Event) -> Action {
        info!("Handling event {event:?}");

        match event {
            Event::StartShutdown => Action::Halt,

            Event::IsOnline(online) => {
                self.is_online = *online;
                Action::Continue
            }

            Event::UploadScreenshot(path, directory) => {
                if let Err(e) = self.upload_screenshot(path, directory).await {
                    error!("Could not upload {path:?}: {e:?}");
                    return Action::Retry;
                }

                if let Err(e) = remove_file(&path).await {
                    self.notify_error(&format!(
                        "Could not remove uploaded screenshot file {path:?}: {e:?}"
                    ));
                    return Action::Continue;
                }

                self.notify_success(true, &format!("Uploaded screenshot {path:?}"));
                Action::Continue
            }

            Event::UploadExtra(path) => {
                let directory = self.extra_directory.clone();
                if let Err(e) = self.upload_screenshot(path, &directory).await {
                    error!("Could not upload {path:?}: {e:?}");
                    return Action::Retry;
                }

                if let Err(e) = remove_file(&path).await {
                    self.notify_error(&format!(
                        "Could not remove extra screenshot file {path:?}: {e:?}"
                    ));
                }

                Action::Continue
            }
        }
    }
}
