mod anthropic;
mod config;
mod error;
mod handler;
mod openai;
mod transform;

use axum::{Router, routing::get, routing::post};
use handler::AppState;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = Arc::new(config::load_config());
    let base_url = config::base_url(&config);
    let port = config.port;
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
        .route(
            "/v1/chat/completions",
            post(handler::chat_completions_handler),
        )
        .route("/health", get(handler::health_handler))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind to {}: {}", addr, e));

    tracing::info!("Gateway listening on {}", addr);

    axum::serve(listener, app).await.expect("Server error");
}
