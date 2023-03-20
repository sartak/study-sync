use anyhow::{Context, Result};
use axum::{extract::Query, response::IntoResponse, routing::get, Router};
use log::info;
use serde::Deserialize;

pub async fn launch(address: &std::net::SocketAddr) -> Result<()> {
    let server = axum::Server::try_bind(address)
        .with_context(|| format!("Failed to bind to {address}"))?
        .serve(router().into_make_service());
    info!("Listening on {address}");
    server.await?;
    Ok(())
}

fn router() -> Router {
    Router::new().route("/game", get(game_get))
}

#[derive(Debug, Deserialize)]
struct GameParams {
    event: String,
    file: String,
}

async fn game_get(Query(params): Query<GameParams>) -> impl IntoResponse {
    info!("GET /game {params:?} -> 200");
    format!("Hello, /game event={} file={}", params.event, params.file)
}
