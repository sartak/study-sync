use std::path::PathBuf;

#[derive(Debug, Clone)]
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

impl rusqlite::types::FromSql for Language {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        value.as_str().map(|v| match v {
            "en" => Language::English,
            "ja" => Language::Japanese,
            "can" => Language::Cantonese,
            _ => Language::Other(v.to_owned()),
        })
    }
}
