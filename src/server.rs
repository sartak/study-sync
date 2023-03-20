use anyhow::Result;
use axum::{routing::get, Router};

pub async fn launch(address: &std::net::SocketAddr) -> Result<()> {
    let server = axum::Server::bind(address).serve(router().into_make_service());
    server.await?;
    Ok(())
}

fn router() -> Router {
    Router::new().route("/", get(|| async { "Hello, world!" }))
}
