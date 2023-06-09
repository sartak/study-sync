use crate::{internal::notifier::Notifier, notify, orchestrator};
use anyhow::{anyhow, Context, Result};
use axum::{
    extract::Query,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use log::info;
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
    notify_tx: mpsc::UnboundedSender<notify::Event>,
}

pub fn prepare() -> (ServerPre, mpsc::UnboundedSender<Event>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (ServerPre { rx }, tx)
}

impl ServerPre {
    pub async fn start(
        self,
        address: &std::net::SocketAddr,
        orchestrator_tx: mpsc::UnboundedSender<orchestrator::Event>,
        notify_tx: mpsc::UnboundedSender<notify::Event>,
    ) -> Result<()> {
        let server = Server {
            orchestrator_tx,
            notify_tx,
        };

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
        .route("/online", post(online_post))
        .route("/offline", post(offline_post))
        .route("/sync", post(sync_post))
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
            server.notify_error(&format!("GET /game -> 500 ({e:?})"));
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
        server.notify_error(&format!("GET /game -> 500 ({e:?})"));
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    info!("GET /game -> 204");
    StatusCode::NO_CONTENT.into_response()
}

async fn online_post(State(server): State<Arc<Server>>) -> impl IntoResponse {
    if let Err(e) = server
        .orchestrator_tx
        .send(orchestrator::Event::IsOnline(true))
    {
        let e = anyhow!(e).context("failed to send event to orchestrator");
        server.notify_error(&format!("POST /online -> 500 ({e:?})"));
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    info!("POST /online -> 204");
    StatusCode::NO_CONTENT.into_response()
}

async fn offline_post(State(server): State<Arc<Server>>) -> impl IntoResponse {
    if let Err(e) = server
        .orchestrator_tx
        .send(orchestrator::Event::IsOnline(false))
    {
        let e = anyhow!(e).context("failed to send event to orchestrator");
        server.notify_error(&format!("POST /offline -> 500 ({e:?})"));
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    info!("POST /offline -> 204");
    StatusCode::NO_CONTENT.into_response()
}

async fn sync_post(State(server): State<Arc<Server>>) -> impl IntoResponse {
    if let Err(e) = server.orchestrator_tx.send(orchestrator::Event::ForceSync) {
        let e = anyhow!(e).context("failed to send event to orchestrator");
        server.notify_error(&format!("POST /sync -> 500 ({e:?})"));
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    info!("POST /sync -> 204");
    StatusCode::NO_CONTENT.into_response()
}

impl Notifier for Server {
    fn notify_target(&self) -> &str {
        "study_sync::server"
    }

    fn notify_tx(&self) -> &mpsc::UnboundedSender<notify::Event> {
        &self.notify_tx
    }
}
