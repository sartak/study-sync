use anyhow::Result;
use log::info;
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
