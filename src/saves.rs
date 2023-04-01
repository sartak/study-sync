use crate::{
    notify::{self, Notifier},
    orchestrator,
    screenshots::{make_retry, Online, Uploader},
};
use anyhow::Result;
use log::{error, info};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use tokio::fs::remove_file;
use tokio::sync::mpsc;
use tokio::time::timeout_at;

#[derive(Debug)]
pub enum Event {
    UploadSave(PathBuf, PathBuf),
    UploadScreenshot(PathBuf, PathBuf),
    IsOnline(bool),
    StartShutdown,
}

pub struct SavesPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Saves {
    rx: mpsc::UnboundedReceiver<Event>,
    orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
    notify_tx: mpsc::UnboundedSender<notify::Event>,
    save_url: String,
    digest_cache: Option<(PathBuf, String)>,
    is_online: bool,
}

pub fn prepare() -> (SavesPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (SavesPre { rx }, tx)
}

impl SavesPre {
    pub async fn start(
        self,
        orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
        notify_tx: mpsc::UnboundedSender<notify::Event>,
        save_url: String,
        is_online: bool,
    ) -> Result<()> {
        let saves = Saves {
            rx: self.rx,
            orchestrator_tx,
            notify_tx,
            save_url,
            digest_cache: None,
            is_online,
        };
        saves.start().await
    }
}

impl Saves {
    pub async fn start(mut self) -> Result<()> {
        let mut retry_deadline = None;
        let mut buffer = VecDeque::new();

        loop {
            // If the buffer is empty, block until we get an event
            let event = if buffer.is_empty() {
                self.rx.recv().await
            // Otherwise we have events to process. First let's see if we have
            // a deadline to wait for; if so then we'll block on the channel
            // until the deadline
            } else if let Some((online_deadline, offline_deadline)) = retry_deadline {
                let deadline = if self.is_online {
                    online_deadline
                } else {
                    offline_deadline
                };
                info!("Waiting until {deadline:?} to retry");
                match timeout_at(deadline, self.rx.recv()).await {
                    Ok(event) => event,
                    Err(_) => {
                        retry_deadline = None;
                        None
                    }
                }
            // Otherwise we have an event to process but no deadline to wait for
            // so just quickly check the channel (which will almost certainly be
            // empty) then proceed to processing events
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

                    Event::IsOnline(online) => self.is_online = online,

                    _ => buffer.push_back(event),
                }
            } else if let Some(event) = buffer.pop_front() {
                match &event {
                    Event::UploadSave(path, directory) => {
                        if let Err(e) = self.upload_file(path, directory, false).await {
                            error!("Could not upload {path:?}: {e:?}");
                            buffer.push_front(event);
                            retry_deadline = make_retry();
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
                            buffer.push_front(event);
                            retry_deadline = make_retry();
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

                    Event::IsOnline(_) => unreachable!(),

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

impl Online for Saves {
    fn orchestrator_tx(&self) -> &mpsc::UnboundedSender<orchestrator::Event> {
        &self.orchestrator_tx
    }

    fn is_online(&self) -> bool {
        self.is_online
    }
}
