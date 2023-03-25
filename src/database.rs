use crate::game::{Game, Play};
use anyhow::Result;
use log::info;
use rusqlite::params;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::join;
use tokio_rusqlite::Connection;

pub struct Database {
    plays_dbh: Connection,
    games_dbh: Connection,
}

pub async fn connect<P>(plays_path: P, games_path: P) -> Result<Database>
where
    P: AsRef<std::path::Path> + std::fmt::Debug,
{
    let plays_dbh =
        Connection::open_with_flags(&plays_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE);
    let games_dbh =
        Connection::open_with_flags(&games_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY);

    let (plays_dbh, games_dbh) = join!(plays_dbh, games_dbh);

    let plays_dbh = plays_dbh?;
    let games_dbh = games_dbh?;
    info!("Connected to databases (plays {plays_path:?}, games {games_path:?})");

    Ok(Database {
        plays_dbh,
        games_dbh,
    })
}

impl Database {
    pub async fn game_for_path(self: &Self, path: &Path) -> Result<Game> {
        let path = PathBuf::from(path);
        self.games_dbh
            .call(|conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT rowid, directory, language, label FROM games WHERE path = ?",
                )?;

                let path_param = path.clone();
                Ok(stmt.query_row(params![&path_param.to_str()], |row| {
                    Ok(Game {
                        id: row.get(0)?,
                        path,
                        directory: row.get(1)?,
                        language: row.get(2)?,
                        label: row.get(3)?,
                    })
                })?)
            })
            .await
    }

    pub async fn started_playing(self: &Self, game: Game) -> Result<Play> {
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.plays_dbh
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO plays (game, start_time) VALUES (?, ?)",
                    params![game.path.to_str(), start_time],
                )?;
                let id = conn.last_insert_rowid();
                Ok(Play {
                    id,
                    game,
                    start_time,
                    end_time: None,
                    intake_id: None,
                    submitted_start: None,
                    submitted_end: None,
                    skipped: false,
                })
            })
            .await
    }

    pub async fn finished_playing(self: &Self, play: Play) -> Result<Play> {
        let end_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.plays_dbh
            .call(move |conn| {
                conn.execute(
                    "UPDATE plays SET end_time=? WHERE rowid=?",
                    params![end_time, play.id],
                )?;
                Ok(Play {
                    end_time: Some(end_time),
                    ..play
                })
            })
            .await
    }

    pub async fn save_currently_playing(self: &Self, id: Option<i64>) -> Result<()> {
        self.plays_dbh
            .call(move |conn| {
                conn.execute("DELETE FROM current", params![])?;
                if let Some(id) = id {
                    conn.execute("INSERT INTO current (play) VALUES (?)", params![id])?;
                }
                Ok(())
            })
            .await
    }
}
