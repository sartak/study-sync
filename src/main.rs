mod event;
mod game;
mod orchestrator;
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

    #[clap(long, required = true, num_args = 1.., value_delimiter = ',')]
    watch_screenshots: Vec<PathBuf>,

    #[clap(flatten)]
    verbose: clap_verbosity_flag::Verbosity,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    env_logger::Builder::new()
        .filter_level(args.verbose.log_level_filter())
        .init();

    let listen = args.listen.parse().unwrap();

    let (orchestrator, tx) = orchestrator::launch();
    let server = server::launch(&listen, tx.clone());
    let screenshots = watch::launch(
        args.watch_screenshots,
        watch::WatchTarget::Screenshots,
        tx.clone(),
    );
    let handler = orchestrator.start();

    select! {
        res = server => { res }
        res = screenshots => { res }
        res = handler => { res }
    }
}
