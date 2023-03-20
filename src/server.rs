use crate::event::Event;
use anyhow::{anyhow, Context, Result};
use axum::{
    extract::Query, extract::State, http::StatusCode, response::IntoResponse, routing::get, Router,
};
use log::{error, info};
use serde::Deserialize;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub async fn launch(
    address: &std::net::SocketAddr,
    tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    let server = axum::Server::try_bind(address)
        .with_context(|| format!("Failed to bind to {address}"))?
        .serve(router(tx).into_make_service());
    info!("Listening on {address}");
    server.await?;
    Ok(())
}

fn router(tx: mpsc::UnboundedSender<Event>) -> Router {
    Router::new()
        .route("/game", get(game_get))
        .route("/shutdown", get(shutdown_get))
        .with_state(tx)
}

#[derive(Debug, Deserialize)]
struct GameParams {
    event: String,
    file: PathBuf,
}

async fn game_get(
    Query(params): Query<GameParams>,
    State(tx): State<mpsc::UnboundedSender<Event>>,
) -> impl IntoResponse {
    let event = match params.event.as_str() {
        "start" => Event::GameStarted(params.file),
        "end" => Event::GameEnded(params.file),
        _ => {
            info!(
                "GET /game {params:?} -> 400 (invalid event: {})",
                params.event
            );
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid event: {}", params.event),
            )
                .into_response();
        }
    };

    if let Err(e) = tx.send(event) {
        let e = anyhow!(e).context("failed to send event on channel");
        error!("GET /game -> 500 ({e:?})");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    info!("GET /game -> 204");
    StatusCode::NO_CONTENT.into_response()
}

async fn shutdown_get(State(tx): State<mpsc::UnboundedSender<Event>>) -> impl IntoResponse {
    if let Err(e) = tx.send(Event::StartShutdown) {
        let e = anyhow!(e).context("failed to send event on channel");
        error!("GET /shutdown -> 500 ({e:?})");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // todo: block until shutdown completed

    info!("GET /shutdown -> 204");
    StatusCode::NO_CONTENT.into_response()
}
