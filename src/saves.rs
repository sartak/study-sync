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
    UploadSave(PathBuf, PathBuf),
    UploadScreenshot(PathBuf, PathBuf),
    IsOnline(bool),
    ForceSync,
    StartShutdown,
}

pub struct SavesPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Saves {
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
            orchestrator_tx,
            notify_tx,
            save_url,
            digest_cache: None,
            is_online,
        };
        saves.start(self.rx).await
    }
}

impl Saves {
    pub async fn start(mut self, rx: mpsc::UnboundedReceiver<Event>) -> Result<()> {
        self.run(rx).await;
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

#[async_trait]
impl PriorityRetryChannel for Saves {
    type Event = Event;

    fn is_online(&self) -> bool {
        self.is_online
    }

    fn is_high_priority(&self, event: &Event) -> bool {
        match event {
            Event::StartShutdown => true,
            Event::IsOnline(_) => true,
            Event::ForceSync => true,

            Event::UploadSave(_, _) => false,
            Event::UploadScreenshot(_, _) => false,
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

            Event::ForceSync => {
                self.is_online = true;
                Action::ResetTimeout
            }

            Event::UploadSave(path, directory) => {
                if let Err(e) = self.upload_file(path, directory, false).await {
                    error!("Could not upload {path:?}: {e:?}");
                    return Action::Retry;
                }

                if let Err(e) = remove_file(&path).await {
                    self.notify_error(&format!(
                        "Could not remove uploaded save file {path:?}: {e:?}"
                    ));
                    return Action::Continue;
                }

                self.notify_success(true, &format!("Uploaded save {path:?}"));

                Action::Continue
            }

            Event::UploadScreenshot(path, directory) => {
                if let Err(e) = self.upload_file(path, directory, true).await {
                    error!("Could not upload {path:?}: {e:?}");
                    return Action::Retry;
                }

                if let Err(e) = remove_file(&path).await {
                    self.notify_error(&format!(
                        "Could not remove uploaded save screenshot file {path:?}: {e:?}"
                    ));
                    return Action::Continue;
                }

                self.notify_success(true, &format!("Uploaded save screenshot {path:?}"));

                Action::Continue
            }
        }
    }
}
