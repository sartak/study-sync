use std::path::PathBuf;

#[derive(Debug)]
pub enum Language {
    English,
    Japanese,
    Cantonese,
    Other(String),
}

#[derive(Debug)]
pub struct Game {
    pub id: i64,
    pub path: PathBuf,
    pub directory: String,
    pub language: Language,
    pub label: String,
}

#[derive(Debug)]
pub struct Play {
    pub id: i64,
    pub game: Game,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub intake_id: Option<u64>,
    pub submitted_start: Option<u64>,
    pub submitted_end: Option<u64>,
    pub skipped: bool,
}
