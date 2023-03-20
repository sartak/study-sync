use std::path::PathBuf;

#[derive(Debug)]
pub enum Event {
    GameStarted(PathBuf),
    GameEnded(PathBuf),
    ScreenshotCreated(PathBuf),
    SaveFileCreated(PathBuf),
    StartShutdown,
}
