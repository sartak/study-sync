use crate::{internal::notifier::Notifier, notify, orchestrator};
use anyhow::{Result, anyhow};
use axum::{
    Router,
    extract::{Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{fs::canonicalize, sync::mpsc};
use tower_http::trace::TraceLayer;
use tracing::{Span, info, info_span, warn};

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

        let listener = tokio::net::TcpListener::bind(address).await?;
        let listener = axum::serve(listener, router(server).into_make_service())
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
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    info_span!(
                        "http_request",
                        method = ?request.method(),
                        path = ?request.uri(),
                        duration_ms = tracing::field::Empty,
                        status = tracing::field::Empty,
                    )
                })
                .on_response(|response: &Response, duration: Duration, span: &Span| {
                    let duration = (duration.as_micros() as f64) / 1000.0;
                    span.record("duration_ms", duration);
                    span.record("status", response.status().as_u16());
                    if response.status().is_success() {
                        info!("{}", response.status());
                    } else {
                        warn!("{}", response.status());
                    }
                }),
        )
}

#[derive(Debug, Deserialize)]
struct GameParams {
    event: String,
    file: PathBuf,
}

async fn game_get(Query(params): Query<GameParams>, State(server): State<Arc<Server>>) -> Response {
    let file = match canonicalize(params.file).await {
        Ok(f) => f,
        Err(e) => {
            let e = anyhow!(e).context("failed to canonicalize path");
            server.notify_error(&e.to_string());
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let event = match params.event.as_str() {
        "start" => orchestrator::Event::GameStarted(file),
        "end" => orchestrator::Event::GameEnded(file),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid event: {}", params.event),
            )
                .into_response();
        }
    };

    if let Err(e) = server.orchestrator_tx.send(event) {
        let e = anyhow!(e).context("failed to send event to orchestrator");
        server.notify_error(&e.to_string());
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::NO_CONTENT.into_response()
}

async fn online_post(State(server): State<Arc<Server>>) -> Response {
    if let Err(e) = server
        .orchestrator_tx
        .send(orchestrator::Event::IsOnline(true))
    {
        let e = anyhow!(e).context("failed to send event to orchestrator");
        server.notify_error(&e.to_string());
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::NO_CONTENT.into_response()
}

async fn offline_post(State(server): State<Arc<Server>>) -> Response {
    if let Err(e) = server
        .orchestrator_tx
        .send(orchestrator::Event::IsOnline(false))
    {
        let e = anyhow!(e).context("failed to send event to orchestrator");
        server.notify_error(&e.to_string());
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::NO_CONTENT.into_response()
}

async fn sync_post(State(server): State<Arc<Server>>) -> Response {
    if let Err(e) = server.orchestrator_tx.send(orchestrator::Event::ForceSync) {
        let e = anyhow!(e).context("failed to send event to orchestrator");
        server.notify_error(&e.to_string());
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::NO_CONTENT.into_response()
}

impl Notifier for Server {
    fn notify_tx(&self) -> &mpsc::UnboundedSender<notify::Event> {
        &self.notify_tx
    }
}
