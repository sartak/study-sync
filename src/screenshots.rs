use crate::{
    channel::{self, Action, PriorityRetryChannel},
    notify::{self, Notifier},
    orchestrator,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::{error, info};
use reqwest::Body;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs::{remove_file, File};
use tokio::sync::mpsc;
use tokio_util::codec::{BytesCodec, FramedRead};

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

#[async_trait]
pub trait Uploader: Notifier + Send + Online {
    fn get_digest_cache(&self) -> &Option<(PathBuf, String)>;
    fn set_digest_cache(&mut self, cache: Option<(PathBuf, String)>);

    async fn digest_for_path(&mut self, path: &Path) -> Option<String> {
        if let Some((p, d)) = self.get_digest_cache() {
            if p == path {
                return Some(d.clone());
            }
        }

        let res = {
            let path = path.to_owned();
            tokio::task::spawn_blocking(move || -> Result<String> {
                let mut file = std::fs::File::open(path)?;
                let mut hasher = Sha1::new();
                std::io::copy(&mut file, &mut hasher)?;
                Ok(hex::encode(hasher.finalize()))
            })
        }
        .await;

        match res {
            Ok(Ok(digest)) => {
                self.set_digest_cache(Some((path.to_path_buf(), digest.clone())));
                Some(digest)
            }
            Ok(Err(e)) => {
                self.notify_error(&format!("Could not calculate digest of {path:?}: {e:?}"));
                None
            }
            Err(e) => {
                self.notify_error(&format!(
                    "Could not join future for calculating digest of {path:?}: {e:?}"
                ));
                None
            }
        }
    }

    async fn upload_path_to_directory(
        &mut self,
        base_url: &str,
        path: &Path,
        directory: &str,
        content_type: Option<&str>,
    ) -> Result<()> {
        let mut url = format!("{base_url}/{directory}");

        let basename = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("");

        if let Some(digest) = self.digest_for_path(path).await {
            let param = format!("?digest={digest}");
            url.push_str(&param);
        }

        let file = File::open(path).await?;
        let stream = FramedRead::new(file, BytesCodec::new());
        let body = Body::wrap_stream(stream);

        let builder = reqwest::ClientBuilder::new().timeout(Duration::from_secs(30));
        let client = builder.build()?;

        let mut req = client
            .post(&url)
            .header("X-Study-Basename", basename)
            .body(body);

        if let Some(content_type) = content_type {
            req = req.header(reqwest::header::CONTENT_TYPE, content_type);
        }

        let res = match req.send().await {
            Ok(res) => res,
            Err(e) => {
                self.observed_error(&e);
                return Err(anyhow!(e));
            }
        };

        self.observed_online();

        if !res.status().is_success() {
            return Err(anyhow!(
                "Failed to upload {path:?} using {url:?}: got status code {}",
                res.status()
            ));
        }

        let message = res.text().await?;
        info!("Successfully uploaded {path:?} to {url:?}: {message}");

        Ok(())
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

pub trait Online: Notifier {
    fn orchestrator_tx(&self) -> &mpsc::UnboundedSender<orchestrator::Event>;
    fn is_online(&self) -> bool;

    fn observed_is_online(&self, is_online: bool) {
        if self.is_online() == is_online {
            return;
        }

        info!(
            "Observed online status changed: {:?} -> {is_online:?}",
            self.is_online()
        );

        if let Err(e) = self
            .orchestrator_tx()
            .send(orchestrator::Event::IsOnline(is_online))
        {
            self.notify_error(&format!("Could not send to orchestrator: {e:?}"));
        }
    }

    fn observed_online(&self) {
        self.observed_is_online(true);
    }

    fn observed_offline(&self) {
        self.observed_is_online(false);
    }

    fn observed_error(&self, error: &reqwest::Error) {
        if error.is_timeout() || format!("{error:?}").contains("error trying to connect") {
            self.observed_offline();
        }
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
impl channel::PriorityRetryChannel for Screenshots {
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
