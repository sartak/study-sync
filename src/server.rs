use crate::orchestrator;
use anyhow::{anyhow, Context, Result};
use axum::{
    extract::Query, extract::State, http::StatusCode, response::IntoResponse, routing::get, Router,
};
use log::{error, info};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::{fs::canonicalize, sync::mpsc};

#[derive(Debug)]
pub enum Event {
    StartShutdown,
}

pub struct ServerPre {
    rx: mpsc::UnboundedReceiver<Event>,
}

pub struct Server {
    orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
}

pub fn launch() -> (ServerPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (ServerPre { rx }, tx)
}

impl ServerPre {
    pub async fn start(
        self,
        address: &std::net::SocketAddr,
        orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
    ) -> Result<()> {
        let server = Server { orchestrator_tx };

        let listener = axum::Server::try_bind(address)
            .with_context(|| format!("Failed to bind to {address}"))?
            .serve(router(server).into_make_service())
            .with_graceful_shutdown(handle_events(self.rx));
        info!("Listening on {address}");
        listener.await?;
        Ok(())
    }
}

async fn handle_events(mut rx: mpsc::UnboundedReceiver<Event>) {
    #![allow(clippy::never_loop)]
    while let Some(event) = rx.recv().await {
        info!("Handling event {event:?}");
        match event {
            Event::StartShutdown => break,
        }
    }

    info!("server gracefully shutting down");
}

fn router(server: Server) -> Router {
    Router::new()
        .route("/game", get(game_get))
        .route("/shutdown", get(shutdown_get))
        .with_state(Arc::new(server))
}

#[derive(Debug, Deserialize)]
struct GameParams {
    event: String,
    file: PathBuf,
}

async fn game_get(
    Query(params): Query<GameParams>,
    State(server): State<Arc<Server>>,
) -> impl IntoResponse {
    let file = match canonicalize(params.file).await {
        Ok(f) => f,
        Err(e) => {
            let e = anyhow!(e).context("failed to canonicalize path");
            error!("GET /game -> 500 ({e:?})");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let event = match params.event.as_str() {
        "start" => orchestrator::Event::GameStarted(file),
        "end" => orchestrator::Event::GameEnded(file),
        _ => {
            info!(
                "GET /game (file {:?}) -> 400 (invalid event: {})",
                file, params.event
            );
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid event: {}", params.event),
            )
                .into_response();
        }
    };

    if let Err(e) = server.orchestrator_tx.send(event) {
        let e = anyhow!(e).context("failed to send event to orchestrator");
        error!("GET /game -> 500 ({e:?})");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    info!("GET /game -> 204");
    StatusCode::NO_CONTENT.into_response()
}

async fn shutdown_get(State(server): State<Arc<Server>>) -> impl IntoResponse {
    if let Err(e) = server
        .orchestrator_tx
        .send(orchestrator::Event::StartShutdown)
    {
        let e = anyhow!(e).context("failed to send event on channel");
        error!("GET /shutdown -> 500 ({e:?})");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // todo: block until shutdown completed

    info!("GET /shutdown -> 204");
    StatusCode::NO_CONTENT.into_response()
}
