mod server;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    server::launch(&"0.0.0.0:3000".parse().unwrap()).await?;
    Ok(())
}
