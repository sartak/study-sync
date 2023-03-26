use crate::game::{Game, Play};
use anyhow::Result;
use log::{error, info};
use rusqlite::{params, OptionalExtension};
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

async fn save_currently_playing(dbh: Connection, id: Option<i64>) -> Result<()> {
    dbh.call(move |conn| {
        conn.execute("DELETE FROM current", [])?;
        if let Some(id) = id {
            conn.execute("INSERT INTO current (play) VALUES (?)", params![id])?;
        }
        Ok(())
    })
    .await
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

    pub fn detach_save_currently_playing(self: &Self, id: Option<i64>) {
        let db = self.plays_dbh.clone();
        tokio::spawn(async move {
            if let Err(e) = save_currently_playing(db, id).await {
                error!("Error saving currently playing: {e:?}")
            }
        });
    }

    pub async fn load_previously_playing(self: &Self) -> Result<Option<Play>> {
        struct Current {
            rowid: i64,
            game_path: String,
            start_time: u64,
            end_time: Option<u64>,
            intake_id: Option<u64>,
            submitted_start: Option<u64>,
            submitted_end: Option<u64>,
            skipped: bool,
        }

        let current: Option<Current> = self
            .plays_dbh
            .call(|conn| {
                let mut stmt = conn.prepare_cached("SELECT rowid, game, start_time, end_time, intake_id, submitted_start, submitted_end, skipped FROM plays WHERE rowid = (SELECT play FROM current)")?;

                let current = stmt.query_row([], |row| Ok(Current {
                    rowid: row.get(0)?,
                    game_path: row.get(1)?,
                    start_time: row.get(2)?,
                    end_time: row.get(3)?,
                    intake_id: row.get(4)?,
                    submitted_start: row.get(5)?,
                    submitted_end: row.get(6)?,
                    skipped: row.get(7)?,
                })).optional()?;

                Ok::<_, rusqlite::Error>(current)
            })
            .await?;
        let current = match current {
            Some(c) => c,
            None => return Ok(None),
        };

        let game = self
            .game_for_path(&PathBuf::from(current.game_path))
            .await?;

        Ok(Some(Play {
            id: current.rowid,
            game,
            start_time: current.start_time,
            end_time: current.end_time,
            intake_id: current.intake_id,
            submitted_start: current.submitted_start,
            submitted_end: current.submitted_end,
            skipped: current.skipped,
        }))
    }
}
