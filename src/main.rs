mod server;
mod watch;

use anyhow::Result;
use clap::Parser;
use log::error;
use tokio::select;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long)]
    address: String,

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

    let server = server::launch(&address);
    let watch = watch::launch("/home/shawn/tmp");

    select! {
        res = server => {
            if let Err(e) = res {
                error!("Error in server: {}", e)
            }
        }
        res = watch => {
            if let Err(e) = res {
                error!("Error in watcher: {}", e)
            }
        }
    }

    Ok(())
}
