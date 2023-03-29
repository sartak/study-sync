mod database;
mod intake;
mod notify;
mod orchestrator;
mod screenshots;
mod server;
mod watch;

use anyhow::{anyhow, Result};
use clap::Parser;
use log::{error, info};
use std::{path::Path, path::PathBuf, process};
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
    extra_directory: String,

    #[clap(long, required = true, num_args = 1.., value_delimiter = ',')]
    watch_screenshots: Vec<PathBuf>,

    #[arg(long)]
    hold_screenshots: PathBuf,

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
    if !args.hold_screenshots.is_dir() {
        return Err(anyhow!(
            "hold-screenshots {:?} not a directory",
            args.hold_screenshots
        ));
    }

    let (server, server_tx) = server::prepare();
    let (watch, watch_tx) = watch::prepare();
    let (orchestrator, orchestrator_tx) = orchestrator::prepare();
    let (intake, intake_tx) = intake::prepare();
    let (screenshots, screenshots_tx) = screenshots::prepare();
    let (notify, notify_tx) = notify::prepare();

    let dbh =
        database::connect(args.plays_database, args.games_database, notify_tx.clone()).await?;

    let server = server.start(&listen, orchestrator_tx.clone(), notify_tx.clone());
    let watch = watch.start(
        args.watch_screenshots.clone(),
        watch::WatchTarget::Screenshots,
        orchestrator_tx.clone(),
        notify_tx.clone(),
    );
    let orchestrator = orchestrator.start(
        dbh,
        args.hold_screenshots,
        args.watch_screenshots,
        args.trim_game_prefix,
        intake_tx,
        screenshots_tx,
        watch_tx,
        server_tx,
        notify_tx.clone(),
    );
    let intake = intake.start(orchestrator_tx.clone(), notify_tx.clone(), args.intake_url);
    let screenshots =
        screenshots.start(notify_tx.clone(), args.screenshot_url, args.extra_directory);
    let notify = notify.start(args.led_path.clone());
    let signal = shutdown_signal(orchestrator_tx);

    let res = try_join!(
        server,
        watch,
        orchestrator,
        intake,
        screenshots,
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
