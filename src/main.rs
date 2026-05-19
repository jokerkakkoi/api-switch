mod anthropic;
mod api;
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
    let port = config.port;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .no_proxy()
        .build()
        .expect("Failed to create HTTP client");

    let state = AppState { config, client };

    let app = Router::new()
        .route("/v1/messages", post(api::messages::anthropic_proxy_handler))
        .route(
            "/v1/chat/completions",
            post(api::chat_completions::openai_passthrough_handler),
        )
        .route("/health", get(api::health::health_handler))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind to {}: {}", addr, e));

    tracing::info!("Gateway listening on {}", addr);

    axum::serve(listener, app).await.expect("Server error");
}
