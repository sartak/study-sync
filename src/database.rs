use crate::{
    intake,
    internal::notifier::Notifier,
    notify,
    orchestrator::{Game, Language, Play},
};
use anyhow::Result;
use futures::future::try_join_all;
use itertools::Itertools;
use log::{error, info};
use rusqlite::{params, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::{join, sync::mpsc};
use tokio_rusqlite::Connection;

pub struct Database {
    plays_dbh: Connection,
    games_dbh: Connection,
    notify_tx: mpsc::UnboundedSender<notify::Event>,
}

pub async fn connect<P>(
    plays_path: P,
    games_path: P,
    notify_tx: mpsc::UnboundedSender<notify::Event>,
) -> Result<Database>
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
        notify_tx,
    })
}

async fn save_currently_playing(dbh: Connection, id: Option<i64>) -> Result<()> {
    Ok(dbh
        .call(move |conn| {
            conn.execute("DELETE FROM current", [])?;
            if let Some(id) = id {
                conn.execute("INSERT INTO current (play) VALUES (?)", params![id])?;
            }
            Ok(())
        })
        .await?)
}

impl Database {
    pub async fn game_for_path(&self, path: &Path) -> Result<Game> {
        let path = PathBuf::from(path);
        Ok(self
            .games_dbh
            .call(|conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT rowid, directory, language, label FROM games WHERE path = ?",
                )?;

                let path_param = path.clone();
                stmt.query_row(params![&path_param.to_str()], |row| {
                    Ok(Game {
                        id: row.get(0)?,
                        path,
                        directory: row.get(1)?,
                        language: row.get(2)?,
                        label: row.get(3)?,
                    })
                })
            })
            .await?)
    }

    pub async fn started_playing(&self, game: Game) -> Result<Play> {
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Ok(self
            .plays_dbh
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
            .await?)
    }

    pub async fn finished_playing(&self, play: Play) -> Result<Play> {
        let end_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Ok(self
            .plays_dbh
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
            .await?)
    }

    pub fn detach_save_currently_playing(&self, id: Option<i64>) {
        let db = self.plays_dbh.clone();

        let notify_tx = self.notify_tx().clone();
        tokio::spawn(async move {
            if let Err(e) = save_currently_playing(db, id).await {
                let message = format!("Error saving currently playing: {e:?}");
                error!("{}", message);

                if let Err(e) = notify_tx.send(notify::Event::Error(message.clone())) {
                    error!("Could not send error {message:?} to notify: {e:?}");
                }
            }
        });
    }

    pub async fn load_previously_playing(&self) -> Result<Option<Play>> {
        struct Current {
            rowid: i64,
            game_path: String,
            start_time: u64,
            end_time: Option<u64>,
            intake_id: Option<String>,
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

    pub async fn load_intake_backlog(&self) -> Result<Vec<intake::Event>> {
        struct PartialPlay {
            rowid: i64,
            game_path: String,
            start_time: u64,
            end_time: Option<u64>,
            intake_id: Option<String>,
        }

        let plays = self.plays_dbh.call(|conn| {
            let mut stmt = conn.prepare("SELECT rowid, game, start_time, end_time, intake_id FROM plays WHERE submitted_end IS NULL AND skipped = 0")?;

            let plays = stmt.query_map([], |row| {
                Ok(PartialPlay{
                    rowid: row.get(0)?,
                    game_path: row.get(1)?,
                    start_time: row.get(2)?,
                    end_time: row.get(3)?,
                    intake_id: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;

            Ok::<_, rusqlite::Error>(plays)
        }).await?;

        if plays.is_empty() {
            return Ok(vec![]);
        }

        let games = try_join_all(
            plays
                .iter()
                .map(|p| &p.game_path)
                .unique()
                .map(|p| self.game_for_path(Path::new(p))),
        )
        .await?;

        let games: HashMap<String, Game> = games
            .into_iter()
            .map(|g| (g.path.to_str().unwrap().to_owned(), g))
            .collect();

        Ok(plays
            .into_iter()
            .filter_map(|p| {
                let Game {
                    language,
                    label: game_label,
                    ..
                } = match games.get(&p.game_path) {
                    Some(g) => g,
                    None => {
                        self.notify_error(&format!(
                            "Did not find mapping for game {}",
                            p.game_path
                        ));
                        return None;
                    }
                };

                let language = language.clone();
                let game_label = game_label.clone();

                match p {
                    PartialPlay {
                        end_time: None,
                        intake_id: Some(_),
                        ..
                    } => None,

                    PartialPlay {
                        rowid,
                        start_time,
                        end_time: None,
                        intake_id: None,
                        ..
                    } => Some(intake::Event::SubmitStarted {
                        play_id: rowid,
                        game_label,
                        language,
                        start_time,
                    }),

                    PartialPlay {
                        rowid,
                        end_time: Some(end_time),
                        intake_id: Some(intake_id),
                        ..
                    } => Some(intake::Event::SubmitEnded {
                        play_id: rowid,
                        intake_id,
                        end_time,
                    }),

                    PartialPlay {
                        rowid,
                        start_time,
                        end_time: Some(end_time),
                        intake_id: None,
                        ..
                    } => Some(intake::Event::SubmitFull {
                        play_id: rowid,
                        game_label,
                        language,
                        start_time,
                        end_time,
                    }),
                }
            })
            .collect())
    }

    pub async fn initial_intake(
        &self,
        play_id: i64,
        intake_id: &str,
        submitted_start: u64,
    ) -> Result<()> {
        let intake_id = intake_id.to_string();
        Ok(self
            .plays_dbh
            .call(move |conn| {
                conn.execute(
                    "UPDATE plays SET intake_id=?, submitted_start=? WHERE rowid=?",
                    params![intake_id, submitted_start, play_id],
                )?;
                Ok(())
            })
            .await?)
    }

    pub async fn final_intake(&self, play_id: i64, submitted_end: u64) -> Result<()> {
        Ok(self
            .plays_dbh
            .call(move |conn| {
                conn.execute(
                    "UPDATE plays SET submitted_end=? WHERE rowid=?",
                    params![submitted_end, play_id],
                )?;
                Ok(())
            })
            .await?)
    }

    pub async fn full_intake(
        &self,
        play_id: i64,
        intake_id: &str,
        submitted_start: u64,
        submitted_end: u64,
    ) -> Result<()> {
        let intake_id = intake_id.to_string();
        Ok(self.plays_dbh
            .call(move |conn| {
                conn.execute(
                    "UPDATE plays SET intake_id=?, submitted_start=?, submitted_end=? WHERE rowid=?",
                    params![intake_id, submitted_start, submitted_end, play_id],
                )?;
                Ok(())
            })
            .await?)
    }
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

impl Notifier for Database {
    fn notify_target(&self) -> &str {
        "study_sync::database"
    }

    fn notify_tx(&self) -> &mpsc::UnboundedSender<notify::Event> {
        &self.notify_tx
    }
}
