use anyhow::{anyhow, Result};
use log::{error, info};
use reqwest::Body;
use sha1::{Digest, Sha1};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs::{remove_file, File};
use tokio::sync::mpsc;
use tokio_util::codec::{BytesCodec, FramedRead};

#[derive(Debug)]
pub enum Event {
    UploadScreenshot(PathBuf, String),
    UploadExtra(PathBuf),
    StartShutdown,
}

pub struct ScreenshotsPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Screenshots {
    rx: mpsc::UnboundedReceiver<Event>,
    screenshot_url: String,
    extra_directory: String,
    buffer: VecDeque<Event>,
    digest_cache: Option<(PathBuf, String)>,
}

pub fn launch() -> (ScreenshotsPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (ScreenshotsPre { rx }, tx)
}

impl ScreenshotsPre {
    pub async fn start(self, screenshot_url: String, extra_directory: String) -> Result<()> {
        let screenshots = Screenshots {
            rx: self.rx,
            screenshot_url,
            extra_directory,
            buffer: VecDeque::new(),
            digest_cache: None,
        };
        screenshots.start().await
    }
}

impl Screenshots {
    pub async fn start(mut self) -> Result<()> {
        loop {
            // if we have a buffer, then we want to just check on the channel and continue
            // otherwise block
            let event = if self.buffer.is_empty() {
                self.rx.recv().await
            } else {
                match self.rx.try_recv() {
                    Ok(e) => Some(e),
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                    _ => None,
                }
            };

            if let Some(event) = event {
                info!("Handling event {event:?}");
                match event {
                    Event::StartShutdown => break,
                    _ => self.buffer.push_back(event),
                }
            } else if let Some(event) = self.buffer.pop_front() {
                match &event {
                    Event::UploadScreenshot(path, directory) => {
                        if let Err(e) = self.upload_path_to_directory(path, directory).await {
                            error!("Could not upload {path:?}: {e:?}");
                            self.buffer.push_front(event);
                            info!("Sleeping for 5s before trying again");
                            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                            continue;
                        }

                        if let Err(e) = remove_file(&path).await {
                            error!("Could not remove uploaded screenshot file {path:?}: {e:?}");
                        }
                    }

                    Event::UploadExtra(path) => {
                        let directory = self.extra_directory.clone();
                        if let Err(e) = self.upload_path_to_directory(path, &directory).await {
                            error!("Could not upload {path:?}: {e:?}");
                            self.buffer.push_front(event);
                            info!("Sleeping for 5s before trying again");
                            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                            continue;
                        }

                        if let Err(e) = remove_file(&path).await {
                            error!("Could not remove extra screenshot file {path:?}: {e:?}");
                        }
                    }

                    Event::StartShutdown => unreachable!(),
                }
            }
        }

        info!("screenshots gracefully shut down");
        Ok(())
    }

    async fn digest_for_path(&mut self, path: &Path) -> Option<String> {
        if let Some((p, digest)) = &self.digest_cache {
            if p == path {
                return Some(digest.clone());
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
                self.digest_cache = Some((path.to_owned(), digest.clone()));
                Some(digest)
            }
            Ok(Err(e)) => {
                error!("Could not calculate digest of {path:?}: {e:?}");
                None
            }
            Err(e) => {
                error!("Could not join future for calculating digest of {path:?}: {e:?}");
                None
            }
        }
    }

    async fn upload_path_to_directory(&mut self, path: &Path, directory: &str) -> Result<()> {
        let mut url = format!("{}/{directory}", self.screenshot_url);
        let extension = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("png");
        let content_type = if extension == "jpg" {
            "image/jpeg"
        } else {
            "image/png"
        };

        if let Some(digest) = self.digest_for_path(path).await {
            let param = format!("?screenshot_digest={digest}");
            url.push_str(&param);
        }

        let file = File::open(path).await?;
        let stream = FramedRead::new(file, BytesCodec::new());
        let body = Body::wrap_stream(stream);

        let builder = reqwest::ClientBuilder::new().timeout(Duration::from_secs(30));
        let client = builder.build()?;

        let res = client
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
            .send()
            .await?;

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
