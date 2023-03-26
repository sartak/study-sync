mod database;
mod game;
mod intake;
mod orchestrator;
mod screenshots;
mod server;
mod watch;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tokio::select;

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
    let dbh = database::connect(args.plays_database, args.games_database).await?;

    let (intake, intake_tx) = intake::launch();
    let (screenshots, screenshots_tx) = screenshots::launch();

    let (orchestrator, orchestrator_tx) = orchestrator::launch();

    let server = server::launch(&listen, orchestrator_tx.clone());

    let watch = watch::launch(
        args.watch_screenshots,
        watch::WatchTarget::Screenshots,
        orchestrator_tx.clone(),
    );

    let orchestrator = orchestrator.start(
        dbh,
        args.hold_screenshots,
        args.trim_game_prefix,
        intake_tx,
        screenshots_tx,
    );
    let intake = intake.start(orchestrator_tx.clone(), args.intake_url);
    let screenshots = screenshots.start();

    select! {
        res = server => { res }
        res = watch => { res }
        res = orchestrator => { res }
        res = intake => { res }
        res = screenshots => { res }
    }
}
