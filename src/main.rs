mod database;
mod intake;
mod notify;
mod orchestrator;
mod saves;
mod screenshots;
mod server;
mod watcher;

use anyhow::{anyhow, Result};
use clap::Parser;
use log::{error, info};
use std::{iter, path::Path, path::PathBuf, process};
use tokio::{select, signal, sync::mpsc, try_join};

#[derive(Parser, Debug)]
struct Args {
    #[arg(long)]
    listen: String,

    #[arg(long)]
    plays_database: PathBuf,

    #[arg(long)]
    games_database: PathBuf,

    #[arg(long)]
    trim_game_prefix: Option<String>,

    #[arg(long)]
    intake_url: String,

    #[arg(long)]
    screenshot_url: String,

    #[arg(long)]
    save_url: String,

    #[arg(long)]
    extra_directory: String,

    #[clap(long, required = true, num_args = 1.., value_delimiter = ',')]
    watch_screenshots: Vec<PathBuf>,

    #[clap(long, required = true, num_args = 1.., value_delimiter = ',')]
    watch_saves: Vec<PathBuf>,

    #[arg(long)]
    pending_screenshots: PathBuf,

    #[arg(long)]
    pending_saves: PathBuf,

    #[arg(long)]
    keep_saves: PathBuf,

    #[arg(long)]
    led_path: PathBuf,

    #[clap(flatten)]
    verbose: clap_verbosity_flag::Verbosity,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    env_logger::Builder::new()
        .filter_level(args.verbose.log_level_filter())
        .init();

    if !args.led_path.is_file() {
        return Err(anyhow!("led-path {:?} not a file", args.led_path));
    }

    let listen = args.listen.parse()?;

    for (flag, path) in args
        .watch_screenshots
        .iter()
        .map(|d| ("--watch-screenshots", d))
        .chain(args.watch_saves.iter().map(|d| ("--watch-saves", d)))
        .chain(iter::once((
            "--pending-screenshots",
            &args.pending_screenshots,
        )))
        .chain(iter::once(("--pending-saves", &args.pending_saves)))
        .chain(iter::once(("--keep-saves", &args.keep_saves)))
    {
        if !path.is_dir() {
            return Err(anyhow!("{flag:?} {path:?} is not a directory"));
        }
    }

    let is_online = true;

    let (server, server_tx) = server::prepare();
    let (screenshot_watcher, screenshot_watcher_tx) = watcher::prepare();
    let (save_watcher, save_watcher_tx) = watcher::prepare();
    let (orchestrator, orchestrator_tx) = orchestrator::prepare();
    let (intake, intake_tx) = intake::prepare();
    let (screenshots, screenshots_tx) = screenshots::prepare();
    let (saves, saves_tx) = saves::prepare();
    let (notify, notify_tx) = notify::prepare();

    let dbh =
        database::connect(args.plays_database, args.games_database, notify_tx.clone()).await?;

    let server = server.start(&listen, orchestrator_tx.clone(), notify_tx.clone());
    let screenshot_watcher = screenshot_watcher.start(
        &args.watch_screenshots,
        watcher::WatchTarget::Screenshots,
        orchestrator_tx.clone(),
        notify_tx.clone(),
    );
    let save_watcher = save_watcher.start(
        &args.watch_saves,
        watcher::WatchTarget::SaveFiles,
        orchestrator_tx.clone(),
        notify_tx.clone(),
    );
    let orchestrator = orchestrator.start(
        dbh,
        args.pending_screenshots,
        args.pending_saves,
        args.keep_saves,
        args.trim_game_prefix,
        intake_tx,
        screenshots_tx,
        saves_tx,
        screenshot_watcher_tx,
        save_watcher_tx,
        server_tx,
        notify_tx.clone(),
    );
    let intake = intake.start(
        orchestrator_tx.clone(),
        notify_tx.clone(),
        args.intake_url,
        is_online,
    );
    let screenshots = screenshots.start(
        notify_tx.clone(),
        args.screenshot_url,
        args.extra_directory,
        is_online,
    );
    let saves = saves.start(notify_tx.clone(), args.save_url, is_online);
    let notify = notify.start(args.led_path.clone());
    let signal = shutdown_signal(orchestrator_tx);

    let res = try_join!(
        server,
        screenshot_watcher,
        save_watcher,
        orchestrator,
        intake,
        screenshots,
        saves,
        notify,
        signal
    )
    .map(|_| ());

    if let Err(e) = &res {
        emergency(&format!("fatal error: {e:?}"), &args.led_path, notify_tx).await;
    } else {
        info!("main gracefully shut down");
    }

    res
}

async fn emergency(
    message: &str,
    led_path: &Path,
    notify_tx: mpsc::UnboundedSender<notify::Event>,
) {
    error!("Emergency: {message:?}");

    if notify_tx.is_closed() {
        notify::blink_emergency(led_path).await;
    } else if let Err(e) = notify_tx.send(notify::Event::Emergency(message.to_owned())) {
        error!("Could not send to notify: {e:?}");
    }
}

async fn shutdown_signal(
    orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
) -> Result<()> {
    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())?;
    select!(
        _ = signal::ctrl_c() => info!("Got interrupt signal, shutting down"),
        _ = sigterm.recv() => info!("Got sigterm, shutting down"),
    );

    if let Err(e) = orchestrator_tx.send(orchestrator::Event::StartShutdown) {
        error!("Could not send to orchestrator: {e:?}");
    }

    tokio::spawn(async {
        signal::ctrl_c().await.unwrap();
        error!("Received multiple shutdown signals, exiting now");
        process::exit(1);
    });

    Ok(())
}
