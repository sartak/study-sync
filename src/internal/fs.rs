use anyhow::{anyhow, Context, Result};
use futures::{
    channel::mpsc::{channel, Receiver},
    SinkExt, StreamExt,
};
use log::{debug, info};
use notify::{
    event::{AccessKind, AccessMode},
    Config, Event as NotifyEvent, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub async fn launch<P>(paths: &[P], tx: mpsc::UnboundedSender<PathBuf>) -> Result<()>
where
    P: AsRef<std::path::Path> + std::fmt::Debug + Send,
{
    let (mut watcher, mut rx) = async_watcher()?;

    for path in paths {
        watcher
            .watch(path.as_ref(), RecursiveMode::NonRecursive)
            .with_context(|| format!("watching path {path:?}"))?;
    }
    info!("Watching for changes to {paths:?}");

    while let Some(res) = rx.next().await {
        match res {
            Ok(event) => {
                debug!("file change: {:?}", event);

                for path in event.paths {
                    let event = match event.kind {
                        // create file outside directory, move it in
                        // EventKind::Create(CreateKind::File) => Event::FileSaved(path),

                        // move file within directory
                        // EventKind::Modify(ModifyKind::Name(RenameMode::To)) => Event::FileSaved(path),

                        // rm file
                        // EventKind::Remove(RemoveKind::File) => Event::FileRemoved(path),

                        // create file in directory
                        EventKind::Access(AccessKind::Close(AccessMode::Write)) => path,

                        _ => continue,
                    };

                    tx.send(event)?
                }
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

pub fn full_extension(path: &Path) -> Option<&str> {
    path.file_name().and_then(|b| {
        b.to_str()
            .and_then(|b| b.split_once('.').map(|(_, after)| after))
    })
}

pub fn remove_full_extension(path: &mut PathBuf) {
    if let Some(basename) = path.file_name() {
        let basename = basename.to_owned();
        if let Some(extension) = full_extension(path) {
            if let Some(basename) = basename.to_str() {
                let stem_len = basename.len() - extension.len() - 1;
                if stem_len > 0 {
                    let stem = &basename[0..stem_len];
                    path.set_file_name(stem);
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_full_extension() {
        assert_eq!(full_extension(Path::new("")), None);
        assert_eq!(full_extension(Path::new(".")), None);
        assert_eq!(full_extension(Path::new("..")), None);
        assert_eq!(full_extension(Path::new(".foo")), Some("foo"));
        assert_eq!(full_extension(Path::new("foo")), None);
        assert_eq!(full_extension(Path::new("foo.")), Some(""));
        assert_eq!(full_extension(Path::new("foo..")), Some("."));
        assert_eq!(full_extension(Path::new("foo.state")), Some("state"));
        assert_eq!(full_extension(Path::new("foo..state")), Some(".state"));
        assert_eq!(full_extension(Path::new("foo.state.")), Some("state."));
        assert_eq!(
            full_extension(Path::new("foo.state.auto")),
            Some("state.auto"),
        );
    }

    #[test]
    fn test_remove_full_extension() {
        fn t(path: &str) -> String {
            let mut path = PathBuf::from(path);
            remove_full_extension(&mut path);
            path.to_str().unwrap().to_string()
        }

        assert_eq!(t(""), "");
        assert_eq!(t("foo"), "foo");
        assert_eq!(t("foo.state"), "foo");
        assert_eq!(t("foo.state.auto"), "foo");
        assert_eq!(t("foo."), "foo");
        assert_eq!(t("foo..state"), "foo");
        assert_eq!(t("."), ".");
        assert_eq!(t(".."), "..");
        assert_eq!(t(".foo"), ".foo");
        assert_eq!(t("..foo"), "..foo");
        assert_eq!(t("..f.oo"), "..f.oo");
    }
}

pub fn recursive_files_in<P>(
    directory: P,
    min_depth: Option<usize>,
) -> impl Iterator<Item = PathBuf>
where
    P: AsRef<std::path::Path>,
{
    let mut walker = walkdir::WalkDir::new(directory).sort_by_file_name();

    if let Some(d) = min_depth {
        walker = walker.min_depth(d);
    }

    walker
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
}
