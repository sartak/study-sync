mod event;
mod orchestrator;
mod server;
mod watch;

use anyhow::Result;
use clap::Parser;
use tokio::select;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long)]
    address: String,

    #[arg(long)]
    path: String,

    #[clap(flatten)]
    verbose: clap_verbosity_flag::Verbosity,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    env_logger::Builder::new()
        .filter_level(args.verbose.log_level_filter())
        .init();

    let address = args.address.parse().unwrap();

    let (orchestrator, tx) = orchestrator::launch();
    let server = server::launch(&address);
    let watch = watch::launch(args.path);
    let handler = orchestrator.start();

    select! {
        res = server => { res }
        res = watch => { res }
        res = handler => { res }
    }
}
