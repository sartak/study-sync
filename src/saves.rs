use crate::{
    notify::{self, Notifier},
    screenshots::Uploader,
};
use anyhow::Result;
use log::{error, info};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use tokio::fs::remove_file;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Event {
    UploadSave(PathBuf, PathBuf),
    UploadScreenshot(PathBuf, PathBuf),
    StartShutdown,
}

pub struct SavesPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Saves {
    rx: mpsc::UnboundedReceiver<Event>,
    notify_tx: mpsc::UnboundedSender<notify::Event>,
    save_url: String,
    buffer: VecDeque<Event>,
    digest_cache: Option<(PathBuf, String)>,
}

pub fn prepare() -> (SavesPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (SavesPre { rx }, tx)
}

impl SavesPre {
    pub async fn start(
        self,
        notify_tx: mpsc::UnboundedSender<notify::Event>,
        save_url: String,
    ) -> Result<()> {
        let saves = Saves {
            rx: self.rx,
            notify_tx,
            save_url,
            buffer: VecDeque::new(),
            digest_cache: None,
        };
        saves.start().await
    }
}

impl Saves {
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
                    Event::UploadSave(path, directory) => {
                        if let Err(e) = self.upload_file(path, directory, false).await {
                            error!("Could not upload {path:?}: {e:?}");
                            self.buffer.push_front(event);
                            info!("Sleeping for 5s before trying again");
                            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                            continue;
                        }

                        if let Err(e) = remove_file(&path).await {
                            self.notify_error(&format!(
                                "Could not remove uploaded save file {path:?}: {e:?}"
                            ));
                            continue;
                        }

                        self.notify_success(true, &format!("Uploaded save {path:?}"));
                    }

                    Event::UploadScreenshot(path, directory) => {
                        if let Err(e) = self.upload_file(path, directory, true).await {
                            error!("Could not upload {path:?}: {e:?}");
                            self.buffer.push_front(event);
                            info!("Sleeping for 5s before trying again");
                            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                            continue;
                        }

                        if let Err(e) = remove_file(&path).await {
                            self.notify_error(&format!(
                                "Could not remove uploaded save screenshot file {path:?}: {e:?}"
                            ));
                            continue;
                        }

                        self.notify_success(true, &format!("Uploaded save screenshot {path:?}"));
                    }

                    Event::StartShutdown => unreachable!(),
                }
            }
        }

        info!("saves gracefully shut down");
        Ok(())
    }

    async fn upload_file(
        &mut self,
        path: &Path,
        directory: &Path,
        is_screenshot: bool,
    ) -> Result<()> {
        let extension = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or(if is_screenshot { "png" } else { "unk" });

        let content_type = if is_screenshot {
            Some(if extension == "jpg" {
                "image/jpeg"
            } else {
                "image/png"
            })
        } else {
            None
        };

        let url = self.save_url.clone();
        self.upload_path_to_directory(&url, path, directory.to_str().unwrap(), content_type)
            .await
    }
}

impl Notifier for Saves {
    fn notify_target(&self) -> &str {
        "study_sync::saves"
    }

    fn notify_tx(&self) -> &mpsc::UnboundedSender<notify::Event> {
        &self.notify_tx
    }
}

impl Uploader for Saves {
    fn get_digest_cache(&self) -> &Option<(PathBuf, String)> {
        &self.digest_cache
    }

    fn set_digest_cache(&mut self, cache: Option<(PathBuf, String)>) {
        self.digest_cache = cache;
    }
}
