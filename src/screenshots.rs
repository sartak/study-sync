use anyhow::Result;
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
}

pub struct ScreenshotsPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Screenshots {
    rx: mpsc::UnboundedReceiver<Event>,
    screenshot_url: String,
}

pub fn launch() -> (ScreenshotsPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (ScreenshotsPre { rx }, tx)
}

impl ScreenshotsPre {
    pub async fn start(self, screenshot_url: String) -> Result<()> {
        let screenshots = Screenshots {
            rx: self.rx,
            screenshot_url,
        };
        screenshots.start().await
    }
}

impl Screenshots {
    pub async fn start(mut self) -> Result<()> {
        while let Some(event) = self.rx.recv().await {
            info!("Handling event {event:?}");
            match event {
                Event::UploadScreenshot(path, directory) => {
                    self.upload_path_to_directory(&path, directory).await;
                    if let Err(e) = remove_file(&path).await {
                        error!("Could not remove uploaded screenshot file {path:?}: {e:?}");
                    }
                }
            }
        }
        Ok(())
    }

    async fn upload_path_to_directory(&self, path: &Path, directory: String) -> () {
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

        let res = {
            let path = path.to_owned();
            tokio::task::spawn_blocking(move || -> Result<String> {
                let mut file = std::fs::File::open(&path)?;
                let mut hasher = Sha1::new();
                std::io::copy(&mut file, &mut hasher)?;
                Ok(hex::encode(hasher.finalize()))
            })
            .await
        };
        match res {
            Ok(Ok(digest)) => {
                let param = format!("?screenshot_digest={digest}");
                url.push_str(&param);
            }
            Ok(Err(e)) => {
                error!("Could not calculate digest of {path:?}: {e:?}");
            }
            Err(e) => {
                error!("Could not join future for calculating digest of {path:?}: {e:?}");
            }
        };

        loop {
            match File::open(path).await {
                Ok(file) => {
                    let stream = FramedRead::new(file, BytesCodec::new());
                    let body = Body::wrap_stream(stream);

                    let builder = reqwest::ClientBuilder::new().timeout(Duration::from_secs(30));
                    let client = builder.build().unwrap();
                    match client
                        .post(&url)
                        .header(reqwest::header::CONTENT_TYPE, content_type)
                        .body(body)
                        .send()
                        .await
                    {
                        Ok(res) => {
                            if res.status().is_success() {
                                match res.text().await {
                                    Ok(message) => {
                                        info!(
                                            "Successfully uploaded {path:?} to {url:?}: {message}"
                                        );
                                        break;
                                    }
                                    Err(e) => {
                                        error!("Failed to parse upload response for {path:?} using {url:?} into text: {e:?}");
                                    }
                                }
                            } else {
                                error!(
                                    "Failed to upload {path:?} using {url:?}: got status code {}",
                                    res.status()
                                );
                            }
                        }
                        Err(e) => {
                            error!("Failed to upload {path:?} using {url:?}: {e:?}");
                        }
                    }
                }
                Err(e) => {
                    error!("Could not open {path:?} for reading: {e:?}");
                }
            }

            info!("Sleeping for 5s before trying again");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }
}
