use crate::event::Event;
use anyhow::{anyhow, Context, Result};
use futures::{
    channel::mpsc::{channel, Receiver},
    SinkExt, StreamExt,
};
use log::{debug, info};
use notify::{
    event::{AccessKind, AccessMode, RemoveKind},
    Config, Event as NotifyEvent, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use tokio::sync::mpsc;

pub async fn launch<P>(path: P, tx: mpsc::UnboundedSender<Event>) -> Result<()>
where
    P: AsRef<std::path::Path> + std::fmt::Display,
{
    let (mut watcher, mut rx) = async_watcher()?;

    watcher
        .watch(path.as_ref(), RecursiveMode::Recursive)
        .with_context(|| format!("watching path {path}"))?;
    info!("Watching for changes to {path}");

    while let Some(res) = rx.next().await {
        match res {
            Ok(event) => {
                debug!("file change: {:?}", event);

                if event.paths.is_empty() {
                    continue;
                }

                let mut paths = event.paths;
                let path = paths.swap_remove(0);

                let event = match event.kind {
                    // create file in directory
                    EventKind::Access(AccessKind::Close(AccessMode::Write)) => {
                        Event::FileSaved(path)
                    }

                    // create file outside directory, move it in
                    // EventKind::Create(CreateKind::File) => Event::FileSaved(path),

                    // move file within directory
                    // EventKind::Modify(ModifyKind::Name(RenameMode::To)) => Event::FileSaved(path),

                    // rm file
                    EventKind::Remove(RemoveKind::File) => Event::FileRemoved(path),

                    _ => continue,
                };

                tx.send(event)?
            }
            Err(e) => return Err(anyhow!(e)),
        }
    }

    Ok(())
}

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<NotifyEvent>>)> {
    let (mut tx, rx) = channel(1);
    let watcher = RecommendedWatcher::new(
        move |res| {
            futures::executor::block_on(async {
                tx.send(res).await.unwrap();
            })
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}