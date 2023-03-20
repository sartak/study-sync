use std::path::PathBuf;

#[derive(Debug)]
pub enum Event {
    GameStarted(PathBuf),
    GameEnded(PathBuf),
    FileSaved(PathBuf),
    FileRemoved(PathBuf),
}
