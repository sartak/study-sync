mod database;
mod intake;
mod orchestrator;
mod screenshots;
mod server;
mod watch;

use anyhow::{anyhow, Result};
use clap::Parser;
use std::path::PathBuf;
use tokio::try_join;

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

    #[clap(flatten)]
    verbose: clap_verbosity_flag::Verbosity,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    env_logger::Builder::new()
        .filter_level(args.verbose.log_level_filter())
        .init();

    let listen = args.listen.parse()?;
    if !args.hold_screenshots.is_dir() {
        return Err(anyhow!(
            "hold-screenshots {:?} not a directory",
            args.hold_screenshots
        ));
    }

    let dbh = database::connect(args.plays_database, args.games_database).await?;

    let (server, server_tx) = server::launch();
    let (watch, watch_tx) = watch::launch();
    let (orchestrator, orchestrator_tx) = orchestrator::launch();
    let (intake, intake_tx) = intake::launch();
    let (screenshots, screenshots_tx) = screenshots::launch();

    let server = server.start(&listen, orchestrator_tx.clone());
    let watch = watch.start(
        args.watch_screenshots.clone(),
        watch::WatchTarget::Screenshots,
        orchestrator_tx.clone(),
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
    );
    let intake = intake.start(orchestrator_tx.clone(), args.intake_url);
    let screenshots = screenshots.start(args.screenshot_url, args.extra_directory);

    try_join!(server, watch, orchestrator, intake, screenshots).map(|_| ())
}
