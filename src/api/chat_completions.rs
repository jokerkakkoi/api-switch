use axum::extract::State;
use axum::http::{HeaderMap, HeaderName};
use axum::response::Response;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::error::AppError;
use crate::transform::headers::build_forwarded_headers;

use super::AppState;

const OPENAI_CHAT_PATH: &str = "/v1/chat/completions";

/// POST /v1/chat/completions — transparent OpenAI proxy (no protocol conversion)
pub async fn openai_passthrough_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, AppError> {
    let body_value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| AppError::InvalidRequestError(format!("Invalid request body: {}", e)))?;

    let model_name = body_value
        .get("model")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AppError::InvalidRequestError("Missing 'model' field in request".to_string())
        })?;

    let model = state.config.require_model(model_name)?;

    let mut fwd_headers = build_forwarded_headers(&headers, &model.app_key, &model.app_sign)?;
    fwd_headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );

    let backend_url = format!(
        "{}{}",
        model.base_url.trim_end_matches('/'),
        OPENAI_CHAT_PATH
    );
    tracing::info!("Forwarding OpenAI request {}", &backend_url);

    let response = state
        .client
        .post(&backend_url)
        .headers(fwd_headers)
        .body(body.clone())
        .send()
        .await?;

    let status = axum::http::StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);

    let mut resp_headers = HeaderMap::new();
    for (key, value) in response.headers().iter() {
        if let Ok(name) = HeaderName::from_bytes(key.as_str().as_bytes()) {
            resp_headers.insert(name, value.clone());
        }
    }

    let is_stream = resp_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false);

    if is_stream {
        let byte_stream = response.bytes_stream();
        let (tx, rx) = mpsc::channel::<Result<axum::body::Bytes, reqwest::Error>>(32);

        tokio::spawn(async move {
            let mut stream = byte_stream;
            while let Some(result) = stream.next().await {
                match result {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        tracing::debug!("[/v1/chat/completions SSE] {}", text);
                        let _ = tx.send(Ok(bytes)).await;
                    }
                    Err(e) => {
                        tracing::error!("[/v1/chat/completions SSE] stream error: {}", e);
                        let _ = tx
                            .send(Ok(axum::body::Bytes::from("data: [DONE]\n\n")))
                            .await;
                        return;
                    }
                }
            }
            tracing::info!("OpenAI request completed (streaming)");
        });

        let body = axum::body::Body::from_stream(ReceiverStream::new(rx));
        let mut resp = Response::builder().status(status).body(body)?;
        *resp.headers_mut() = resp_headers;
        Ok(resp)
    } else {
        let body_bytes = response.bytes().await.unwrap_or_default();
        if !status.is_success() {
            tracing::error!(
                status = status.as_u16(),
                request_body = %body,
                body = %String::from_utf8_lossy(&body_bytes),
                "Backend returned non-200 response"
            );
        }
        let mut resp = Response::builder()
            .status(status)
            .body(axum::body::Body::from(body_bytes))?;
        *resp.headers_mut() = resp_headers;
        tracing::info!("OpenAI request completed (non-streaming)");
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::openai_passthrough_handler;
    use axum::http::HeaderMap;
    use axum::{Router, routing::post};
    use reqwest::Client;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    use crate::config::{AppConfig, ModelConfig};

    use super::AppState;

    fn make_test_config(backend_url: String) -> Arc<AppConfig> {
        Arc::new(AppConfig {
            models: vec![
                ModelConfig {
                    name: "qwen35-397b".into(),
                    app_key: "test-key".into(),
                    app_sign: "test-sign".into(),
                    base_url: backend_url.clone(),
                },
                ModelConfig {
                    name: "glm-5".into(),
                    app_key: "test-key-2".into(),
                    app_sign: "test-sign-2".into(),
                    base_url: backend_url.clone(),
                },
            ],
            port: 0,
        })
    }

    #[tokio::test]
    async fn test_chat_completions_function_calling_proxy() {
        let backend_url = start_mock_backend().await;
        let config = make_test_config(backend_url.clone());

        let state = AppState {
            config,
            client: Client::new(),
        };

        let app = Router::new()
            .route("/v1/chat/completions", post(openai_passthrough_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let req_body = json!({
            "model": "qwen35-397b",
            "messages": [
                {
                    "role": "user",
                    "content": "What is the weather like in Boston?"
                }
            ],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get current temperature for a given location.",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "location": {
                                    "type": "string",
                                    "description": "City and country"
                                }
                            },
                            "required": ["location"],
                            "additionalProperties": false
                        },
                        "strict": true
                    }
                }
            ]
        });

        let res = client
            .post(format!("http://{}/v1/chat/completions", proxy_addr))
            .json(&req_body)
            .send()
            .await
            .expect("Request failed");

        assert_eq!(res.status(), 200);
        let res_json: serde_json::Value = res.json().await.unwrap();

        assert_eq!(
            res_json["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "get_weather"
        );
        assert_eq!(res_json["choices"][0]["finish_reason"], "tool_calls");
    }

    async fn start_mock_backend() -> String {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|headers: HeaderMap, _body: String| async move {
                assert_eq!(
                    headers.get("app-key").unwrap().to_str().unwrap(),
                    "test-key"
                );
                assert_eq!(
                    headers.get("app-sign").unwrap().to_str().unwrap(),
                    "test-sign"
                );

                let resp = json!({
                  "id": "chatcmpl-123",
                  "object": "chat.completion",
                  "created": 1731610419,
                  "model": "gpt-4o",
                  "choices": [
                    {
                      "index": 0,
                      "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                          {
                            "id": "call_12345xyz",
                            "type": "function",
                            "function": {
                              "name": "get_weather",
                              "arguments": "{\"location\":\"Boston, MA\"}"
                            }
                          }
                        ]
                      },
                      "finish_reason": "tool_calls"
                    }
                  ]
                });
                axum::Json(resp)
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{}", addr)
    }

    async fn start_mock_backend_check_content_type_explicit() -> String {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|headers: HeaderMap, _body: String| async move {
                let ct = headers.get("content-type");
                assert!(
                    ct.is_some(),
                    "content-type header must be set on outgoing request to backend"
                );
                let ct_str = ct.unwrap().to_str().unwrap();
                assert!(
                    ct_str.contains("application/json"),
                    "content-type should be application/json, got: {}",
                    ct_str
                );
                let resp = json!({
                    "id": "chatcmpl-123",
                    "object": "chat.completion",
                    "created": 1731610419,
                    "model": "gpt-4o",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "hello"},
                        "finish_reason": "stop"
                    }]
                });
                axum::Json(resp)
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn test_chat_completions_sets_content_type_on_outgoing_request() {
        let backend_url = start_mock_backend_check_content_type_explicit().await;
        let config = make_test_config(backend_url);

        let state = AppState {
            config,
            client: Client::new(),
        };

        let app = Router::new()
            .route("/v1/chat/completions", post(openai_passthrough_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let req_body = r#"{"model":"qwen35-397b","messages":[{"role":"user","content":"hi"}]}"#;

        let res = client
            .post(format!("http://{}/v1/chat/completions", proxy_addr))
            .header("content-type", "text/plain")
            .body(req_body)
            .send()
            .await
            .expect("Request failed");

        assert_eq!(res.status(), 200);
    }

    #[tokio::test]
    async fn openai_passthrough_handler_unknown_model_returns_400() {
        let config = Arc::new(AppConfig {
            models: vec![ModelConfig {
                name: "qwen35-397b".into(),
                app_key: "key".into(),
                app_sign: "sign".into(),
                base_url: "http://localhost".into(),
            }],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(openai_passthrough_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/v1/chat/completions", proxy_addr))
            .json(&json!({
                "model": "unknown-model",
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 400);
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Unknown model: unknown-model")
        );
    }

    #[tokio::test]
    async fn openai_passthrough_handler_known_model_forwards_to_correct_base_url() {
        let captured_url: Arc<std::sync::Mutex<Option<String>>> =
            Arc::new(std::sync::Mutex::new(None));
        let captured_url_clone = captured_url.clone();

        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |uri: axum::http::Uri| {
                let captured = captured_url.clone();
                async move {
                    *captured.lock().unwrap() = Some(uri.to_string());
                    let resp = json!({
                        "id": "chatcmpl-123",
                        "object": "chat.completion",
                        "created": 1731610419,
                        "model": "glm-5",
                        "choices": [{
                            "index": 0,
                            "message": {"role": "assistant", "content": "hello"},
                            "finish_reason": "stop"
                        }]
                    });
                    axum::Json(resp)
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let backend_url = format!("http://{}", addr);

        let config = Arc::new(AppConfig {
            models: vec![
                ModelConfig {
                    name: "qwen35-397b".into(),
                    app_key: "key1".into(),
                    app_sign: "sign1".into(),
                    base_url: "http://wrong-host".into(),
                },
                ModelConfig {
                    name: "glm-5".into(),
                    app_key: "key2".into(),
                    app_sign: "sign2".into(),
                    base_url: backend_url.clone(),
                },
            ],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(openai_passthrough_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/v1/chat/completions", proxy_addr))
            .json(&json!({
                "model": "glm-5",
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 200);
        let forwarded_path = captured_url_clone.lock().unwrap().take().unwrap();
        assert_eq!(forwarded_path, "/v1/chat/completions");
    }

    #[tokio::test]
    async fn openai_passthrough_handler_missing_model_returns_400() {
        let config = Arc::new(AppConfig {
            models: vec![ModelConfig {
                name: "qwen35-397b".into(),
                app_key: "key".into(),
                app_sign: "sign".into(),
                base_url: "http://localhost".into(),
            }],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(openai_passthrough_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/v1/chat/completions", proxy_addr))
            .json(&json!({
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 400);
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Missing 'model' field")
        );
    }

    #[tokio::test]
    async fn openai_passthrough_handler_backend_error_returns_error() {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|_headers: HeaderMap, _body: String| async move {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    r#"{"error":{"message":"Internal server error","type":"server_error"}}"#,
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let backend_url = format!("http://{}", addr);

        let config = Arc::new(AppConfig {
            models: vec![ModelConfig {
                name: "qwen35-397b".into(),
                app_key: "key".into(),
                app_sign: "sign".into(),
                base_url: backend_url,
            }],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(openai_passthrough_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/v1/chat/completions", proxy_addr))
            .json(&json!({
                "model": "qwen35-397b",
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 500);
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Internal server error")
        );
    }

    #[tokio::test]
    async fn openai_passthrough_handler_stream_backend_disconnect_sends_error_chunk() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = socket.into_split();

            let mut buf = vec![0u8; 4096];
            let _ = reader.read(&mut buf).await;

            let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            writer.write_all(response).await.unwrap();

            let chunk1 = b"67\r\ndata: {\"id\":\"chatcmpl-123\",\"object\":\"chat.completion.chunk\",\"created\":1731610419,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n\r\n";
            writer.write_all(chunk1).await.unwrap();
            writer.flush().await.unwrap();

            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            drop(writer);
        });

        let backend_url = format!("http://{}", addr);

        let config = Arc::new(AppConfig {
            models: vec![ModelConfig {
                name: "qwen35-397b".into(),
                app_key: "key".into(),
                app_sign: "sign".into(),
                base_url: backend_url,
            }],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(openai_passthrough_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/v1/chat/completions", proxy_addr))
            .json(&json!({
                "model": "qwen35-397b",
                "messages": [{"role": "user", "content": "hi"}],
                "stream": true
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 200);
        let body = res.text().await.unwrap();

        assert!(body.contains("data:"), "should have SSE data");
        assert!(
            body.contains("error") || body.contains("[DONE]"),
            "should signal error or completion on stream disconnect"
        );
    }
}
