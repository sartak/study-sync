use anyhow::Result;
use log::{error, info};
use reqwest::{multipart, Body};
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
    return (ScreenshotsPre { rx }, tx);
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

    fn agent(&self) -> reqwest::Client {
        let builder = reqwest::ClientBuilder::new().timeout(Duration::from_secs(30));
        builder.build().unwrap()
    }

    async fn upload_path_to_directory(&self, path: &Path, directory: String) -> () {
        let url = format!("{}/{directory}", self.screenshot_url);
        loop {
            match File::open(path).await {
                Ok(file) => {
                    let stream = FramedRead::new(file, BytesCodec::new());
                    let file_body = Body::wrap_stream(stream);
                    let part = multipart::Part::stream(file_body);
                    let form = multipart::Form::new().part("screenshot", part);
                    match self.agent().post(&url).multipart(form).send().await {
                        Ok(res) => {
                            if res.status().is_success() {
                                match res.text().await {
                                    Ok(message) => {
                                        info!("Successfully uploaded {path:?} to {directory:?}: {message}");
                                        break;
                                    }
                                    Err(e) => {
                                        error!("Failed to parse upload response for {path:?} into text: {e:?}");
                                    }
                                }
                            } else {
                                error!("Failed to upload {path:?} for {directory:?}: got status code {}", res.status());
                            }
                        }
                        Err(e) => {
                            error!("Failed to upload {path:?} for {directory:?}: {e:?}");
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
