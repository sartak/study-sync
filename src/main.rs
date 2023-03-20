mod server;
use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long)]
    address: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let address = args.address.parse().unwrap();

    let server = server::launch(&address);
    server.await?;

    Ok(())
}
