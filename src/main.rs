mod anthropic;
mod config;
mod error;
mod handler;
mod openai;
mod transform;

use axum::{routing::get, routing::post, Router};
use handler::AppState;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = Arc::new(config::load_config());
    let base_url = config::base_url();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .expect("Failed to create HTTP client");

    let state = AppState {
        config,
        client,
        base_url,
    };

    let app = Router::new()
        .route("/v1/messages", post(handler::messages_handler))
        .route("/health", get(handler::health_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .expect("Failed to bind to port 3000");

    tracing::info!("Gateway listening on 0.0.0.0:3000");

    axum::serve(listener, app).await.expect("Server error");
}
