use axum::Json;
use axum::extract::State;
use serde::Serialize;
use std::collections::HashMap;

use crate::error::AppError;
use crate::handler::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub health: bool,
    pub network: HashMap<String, bool>,
}

/// GET /health — gateway self-health and per-model network checks
pub async fn health_handler(State(state): State<AppState>) -> Result<Json<HealthResponse>, AppError> {
    let health_timeout = std::time::Duration::from_secs(1);

    let mut join_set = tokio::task::JoinSet::new();

    for model in &state.config.models {
        let name = model.name.clone();
        let base_url = model.base_url.trim_end_matches('/').to_string();
        let client = state.client.clone();
        join_set.spawn(async move {
            let reachable = client
                .get(&base_url)
                .timeout(health_timeout)
                .send()
                .await
                .is_ok();
            (name, reachable)
        });
    }

    let mut network = HashMap::new();
    while let Some(result) = join_set.join_next().await {
        if let Ok((name, reachable)) = result {
            network.insert(name, reachable);
        }
    }

    Ok(Json(HealthResponse {
        health: true,
        network,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        routing::get,
    };
    use reqwest::Client;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    use crate::config::{AppConfig, ModelConfig};

    #[tokio::test]
    async fn health_handler_returns_gateway_health_and_network_status() {
        let app = Router::new().route(
            "/",
            get(|| async move { axum::http::StatusCode::OK }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let backend_url = format!("http://{}", addr);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let config = Arc::new(AppConfig {
            models: vec![
                ModelConfig {
                    name: "qwen35-397b".into(),
                    app_key: "key".into(),
                    app_sign: "sign".into(),
                    base_url: backend_url.clone(),
                },
                ModelConfig {
                    name: "glm-5".into(),
                    app_key: "key2".into(),
                    app_sign: "sign2".into(),
                    base_url: backend_url,
                },
            ],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/health", get(health_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .get(format!("http://{}/health", proxy_addr))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 200);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["health"], true);
        assert_eq!(body["network"]["qwen35-397b"], true);
        assert_eq!(body["network"]["glm-5"], true);
    }

    #[tokio::test]
    async fn health_handler_network_false_for_unreachable_model() {
        let config = Arc::new(AppConfig {
            models: vec![
                ModelConfig {
                    name: "qwen35-397b".into(),
                    app_key: "key".into(),
                    app_sign: "sign".into(),
                    base_url: "http://127.0.0.1:1".into(), // unreachable port
                },
            ],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/health", get(health_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .get(format!("http://{}/health", proxy_addr))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 200);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["health"], true);
        assert_eq!(body["network"]["qwen35-397b"], false);
    }

    #[tokio::test]
    async fn health_handler_empty_models_returns_empty_network() {
        let config = Arc::new(AppConfig {
            models: vec![],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/health", get(health_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .get(format!("http://{}/health", proxy_addr))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 200);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["health"], true);
        assert!(body["network"].as_object().unwrap().is_empty());
    }
}
