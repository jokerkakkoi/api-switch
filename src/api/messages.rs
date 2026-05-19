use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response, Sse};
use std::convert::Infallible;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::anthropic::{AnthropicRequest, AnthropicSSEEvent};
use crate::anthropic::{MessageDeltaData, OutputUsage};
use crate::error::AppError;
use crate::handler::{AppState, resolve_model};
use crate::openai::OpenAISSEChunk;
use crate::transform::headers::build_forwarded_headers;
use crate::transform::request::convert_request;
use crate::transform::response::{convert_response, map_openai_error};
use crate::transform::stream::{StreamState, convert_stream_chunk, format_sse};

const OPENAI_CHAT_PATH: &str = "/v1/chat/completions";

/// POST /v1/messages — main protocol conversion endpoint
pub async fn anthropic_proxy_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, AppError> {
    let anthropic_req: AnthropicRequest = serde_json::from_str(&body)
        .map_err(|e| AppError::InvalidRequestError(format!("Invalid request body: {}", e)))?;

    let model = resolve_model(&state, &anthropic_req.model)?;
    let is_stream = anthropic_req.stream;

    let openai_req = convert_request(anthropic_req);
    let openai_req_body = serde_json::to_string(&openai_req).unwrap_or_default();

    let fwd_headers = build_forwarded_headers(&headers, &model.app_key, &model.app_sign)?;

    let backend_url = format!(
        "{}{}",
        model.base_url.trim_end_matches('/'),
        OPENAI_CHAT_PATH
    );
    tracing::info!("Forwarding Anthropic request {}", &backend_url);
    let request = state
        .client
        .post(&backend_url)
        .headers(fwd_headers)
        .json(&openai_req);
    if is_stream {
        let resp = handle_stream_response(request, &openai_req_body).await;
        tracing::info!("Anthropic request completed (streaming)");
        tracing::debug!("Response: {:#?}", resp);
        resp
    } else {
        let resp = handle_non_stream_response(request, &openai_req_body).await;
        tracing::info!("Anthropic request completed (non-streaming)");
        resp
    }
}

async fn handle_non_stream_response(
    request: reqwest::RequestBuilder,
    request_body: &str,
) -> Result<Response, AppError> {
    let response = request.send().await?;
    let status = response.status();
    if status.is_success() {
        let openai_resp: crate::openai::OpenAIResponse = response
            .json()
            .await
            .map_err(|e| AppError::ApiError(format!("Failed to parse backend response: {}", e)))?;
        let anthropic_resp = convert_response(openai_resp)?;
        Ok(Json(anthropic_resp).into_response())
    } else {
        let status_code = status.as_u16();
        let body = response.text().await.unwrap_or_default();
        tracing::error!(
            status = status_code,
            request_body = %request_body,
            body = %body,
            "Backend returned non-200 response"
        );
        Err(map_openai_error(status_code, &body))
    }
}

async fn handle_stream_response(
    request: reqwest::RequestBuilder,
    request_body: &str,
) -> Result<Response, AppError> {
    let response = request.send().await?;
    let status = response.status();

    if !status.is_success() {
        let status_code = status.as_u16();
        let body = response.text().await.unwrap_or_default();
        tracing::error!(
            status = status_code,
            request_body = %request_body,
            body = %body,
            "Backend returned non-200 response"
        );
        return Err(map_openai_error(status_code, &body));
    }

    let (tx, rx) = mpsc::channel::<Result<axum::response::sse::Event, Infallible>>(32);
    let mut byte_stream = response.bytes_stream();

    tokio::spawn(async move {
        let mut state = StreamState::new();
        let mut buffer = String::new();

        while let Some(result) = byte_stream.next().await {
            match result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    buffer.push_str(&text);

                    while let Some(pos) = buffer.find("\n\n") {
                        let line = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();

                        if let Some(data_str) = line
                            .strip_prefix("data: ")
                            .or_else(|| line.strip_prefix("data:"))
                        {
                            let data_str = data_str.trim();
                            if data_str == "[DONE]" {
                                continue;
                            }
                            if let Ok(chunk) = serde_json::from_str::<OpenAISSEChunk>(data_str) {
                                let events = convert_stream_chunk(&chunk, &mut state);
                                for event in events {
                                    let sse_event = axum::response::sse::Event::default()
                                        .data(format_sse(&event));
                                    if tx.send(Ok(sse_event)).await.is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("[/v1/messages SSE] stream error: {}", e);
                    if state.started {
                        if state.content_block_open {
                            let event = axum::response::sse::Event::default().data(format_sse(
                                &AnthropicSSEEvent::ContentBlockStop {
                                    index: state.content_index,
                                },
                            ));
                            let _ = tx.send(Ok(event)).await;
                        }
                        let event = axum::response::sse::Event::default().data(format_sse(
                            &AnthropicSSEEvent::MessageDelta {
                                delta: MessageDeltaData {
                                    stop_reason: "error".to_string(),
                                    stop_sequence: None,
                                },
                                usage: OutputUsage { output_tokens: 0 },
                            },
                        ));
                        let _ = tx.send(Ok(event)).await;
                        let event = axum::response::sse::Event::default()
                            .data(format_sse(&AnthropicSSEEvent::MessageStop));
                        let _ = tx.send(Ok(event)).await;
                    }
                    return;
                }
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    Ok(Sse::new(stream).into_response())
}

#[cfg(test)]
mod tests {
    use super::anthropic_proxy_handler;
    use axum::{Router, routing::post};
    use axum::http::HeaderMap;
    use reqwest::Client;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    use crate::config::{AppConfig, ModelConfig};
    use crate::handler::AppState;

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

    async fn start_mock_backend_for_sse() -> String {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|_headers: HeaderMap, _body: String| async move {
                let stream = futures::stream::iter(vec![
                    Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(
                        "data: {\"id\":\"chatcmpl-123\",\"object\":\"chat.completion.chunk\",\"created\":1731610419,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
                    )),
                    Ok(axum::body::Bytes::from(
                        "data: {\"id\":\"chatcmpl-123\",\"object\":\"chat.completion.chunk\",\"created\":1731610419,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\n",
                    )),
                    Ok(axum::body::Bytes::from(
                        "data: {\"id\":\"chatcmpl-123\",\"object\":\"chat.completion.chunk\",\"created\":1731610419,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    )),
                    Ok(axum::body::Bytes::from("data: [DONE]\n\n")),
                ]);
                (
                    axum::http::StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    axum::body::Body::from_stream(stream),
                )
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
    async fn test_messages_streaming_returns_sse_events() {
        let backend_url = start_mock_backend_for_sse().await;
        let config = make_test_config(backend_url);

        let state = AppState {
            config,
            client: Client::new(),
        };

        let app = Router::new()
            .route("/v1/messages", post(anthropic_proxy_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let req_body = json!({
            "model": "qwen35-397b",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1024,
            "stream": true
        });

        let res = client
            .post(format!("http://{}/v1/messages", proxy_addr))
            .header("anthropic-version", "2023-06-01")
            .json(&req_body)
            .send()
            .await
            .expect("Request failed");

        assert_eq!(res.status(), 200);
        assert!(
            res.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("text/event-stream")
        );

        let body = res.text().await.expect("Failed to read response body");
        assert!(body.contains("event: message_start"));
        assert!(body.contains("event: content_block_start"));
        assert!(body.contains("event: content_block_delta"));
        assert!(body.contains("event: content_block_stop"));
        assert!(body.contains("event: message_delta"));
        assert!(body.contains("event: message_stop"));
    }

    #[tokio::test]
    async fn anthropic_proxy_handler_unknown_model_returns_400() {
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
            .route("/v1/messages", post(anthropic_proxy_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/v1/messages", proxy_addr))
            .header("anthropic-version", "2023-06-01")
            .json(&json!({
                "model": "unknown-model",
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 100
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
    async fn anthropic_proxy_handler_known_model_forwards_with_correct_credentials() {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|headers: HeaderMap, _body: String| async move {
                assert_eq!(
                    headers.get("app-key").unwrap().to_str().unwrap(),
                    "model-specific-key"
                );
                assert_eq!(
                    headers.get("app-sign").unwrap().to_str().unwrap(),
                    "model-specific-sign"
                );
                let resp = json!({
                    "id": "msg_123",
                    "object": "chat.completion",
                    "created": 1731610419,
                    "model": "qwen35-397b",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "hello"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
                });
                axum::Json(resp)
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
                app_key: "model-specific-key".into(),
                app_sign: "model-specific-sign".into(),
                base_url: backend_url.clone(),
            }],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/v1/messages", post(anthropic_proxy_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/v1/messages", proxy_addr))
            .header("anthropic-version", "2023-06-01")
            .json(&json!({
                "model": "qwen35-397b",
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 100
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 200);
    }

    #[tokio::test]
    async fn anthropic_proxy_handler_invalid_json_returns_400() {
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
            .route("/v1/messages", post(anthropic_proxy_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/v1/messages", proxy_addr))
            .header("anthropic-version", "2023-06-01")
            .body("not valid json {{{")
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 400);
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Invalid request body")
        );
    }

    #[tokio::test]
    async fn anthropic_proxy_handler_backend_error_returns_error() {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|_headers: HeaderMap, _body: String| async move {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    r#"{"error":{"message":"Backend failed","type":"server_error"}}"#,
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
            .route("/v1/messages", post(anthropic_proxy_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/v1/messages", proxy_addr))
            .header("anthropic-version", "2023-06-01")
            .json(&json!({
                "model": "qwen35-397b",
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 100
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 502);
        let body: serde_json::Value = res.json().await.unwrap();
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Backend failed")
        );
    }

    #[tokio::test]
    async fn anthropic_proxy_handler_stream_backend_disconnect_emits_error_event() {
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

            let chunk1 = b"10a\r\ndata: {\"id\":\"chatcmpl-123\",\"object\":\"chat.completion.chunk\",\"created\":1731610419,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n\r\n";
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
            .route("/v1/messages", post(anthropic_proxy_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .post(format!("http://{}/v1/messages", proxy_addr))
            .header("anthropic-version", "2023-06-01")
            .json(&json!({
                "model": "qwen35-397b",
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 1024,
                "stream": true
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 200);
        let body = res.text().await.unwrap();

        assert!(
            body.contains("event: message_start"),
            "should have message_start"
        );
        assert!(
            body.contains("event: message_delta"),
            "should have message_delta with stop_reason on stream error"
        );
        assert!(
            body.contains("event: message_stop"),
            "should have message_stop on stream error"
        );
    }
}
