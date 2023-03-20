use anyhow::{Context, Result};
use axum::{routing::get, Router};
use log::info;

pub async fn launch(address: &std::net::SocketAddr) -> Result<()> {
    let server = axum::Server::try_bind(address)
        .with_context(|| format!("Failed to bind to {address}"))?
        .serve(router().into_make_service());
    info!("Listening on {address}");
    server.await?;
    Ok(())
}

fn router() -> Router {
    Router::new().route("/", get(|| async { "Hello, world!" }))
}
