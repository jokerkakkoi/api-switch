use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName};
use axum::response::{IntoResponse, Response, Sse};
use reqwest::Client;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::anthropic::AnthropicRequest;
use crate::config::AppConfig;
use crate::error::AppError;
use crate::openai::OpenAISSEChunk;
use crate::transform::headers::build_forwarded_headers;
use crate::transform::request::convert_request;
use crate::transform::response::{convert_response, map_openai_error};
use crate::transform::stream::{StreamState, convert_stream_chunk, format_sse};

const OPENAI_CHAT_PATH: &str = "/v1/chat/completions";

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub client: Client,
    pub base_url: String,
}

/// POST /v1/messages — main protocol conversion endpoint
pub async fn messages_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, AppError> {
    // 1. Parse Anthropic request
    let anthropic_req: AnthropicRequest = serde_json::from_str(&body)
        .map_err(|e| AppError::InvalidRequestError(format!("Invalid request body: {}", e)))?;

    let is_stream = anthropic_req.stream;

    // 2. Convert request
    let openai_req = convert_request(anthropic_req);

    // 3. Build forwarded headers: skip hop-by-hop, add App-Key + App-Sign from config
    let fwd_headers = build_forwarded_headers(&headers, &state.config.app_key, &state.config.app_sign);

    // 6. Send request to backend
    let backend_url = format!(
        "{}{}",
        state.base_url.trim_end_matches('/'),
        OPENAI_CHAT_PATH
    );
    tracing::info!("开始{}请求{}", is_stream, &backend_url);
    let request = state
        .client
        .post(&backend_url)
        .headers(fwd_headers)
        .json(&openai_req);
    if is_stream {
        handle_stream_response(request).await
    } else {
        handle_non_stream_response(request).await
    }
}

/// Handle non-streaming OpenAI response → Anthropic response
async fn handle_non_stream_response(
    request: reqwest::RequestBuilder,
) -> Result<Response, AppError> {
    let response = request.send().await?;
    let status = response.status();
    if status.is_success() {
        let openai_resp: crate::openai::OpenAIResponse = response
            .json()
            .await
            .map_err(|e| AppError::ApiError(format!("Failed to parse backend response: {}", e)))?;
        let anthropic_resp = convert_response(openai_resp);
        Ok(Json(anthropic_resp).into_response())
    } else {
        let status_code = status.as_u16();
        let body = response.text().await.unwrap_or_default();
        Err(map_openai_error(status_code, &body))
    }
}

/// Handle streaming SSE response: read OpenAI SSE stream, convert to Anthropic SSE, stream to client.
async fn handle_stream_response(request: reqwest::RequestBuilder) -> Result<Response, AppError> {
    let response = request.send().await?;
    let status = response.status();

    if !status.is_success() {
        let status_code = status.as_u16();
        let body = response.text().await.unwrap_or_default();
        return Err(map_openai_error(status_code, &body));
    }

    // Use mpsc channel to bridge the spawned task and the response stream
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

                    // Process complete SSE messages (separated by \n\n)
                    while let Some(pos) = buffer.find("\n\n") {
                        let line = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();

                        // Parse "data: {json}" line
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
                                        return; // client disconnected
                                    }
                                }
                            }
                        }
                    }
                }
                Err(_) => {
                    return;
                }
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    Ok(Sse::new(stream).into_response())
}

/// POST /v1/chat/completions — transparent OpenAI proxy (no protocol conversion)
pub async fn chat_completions_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, AppError> {
    // 1. Build forwarded headers: skip hop-by-hop, add App-Key + App-Sign from config
    let fwd_headers = build_forwarded_headers(&headers, &state.config.app_key, &state.config.app_sign);

    // 2. Forward to backend
    let backend_url = format!(
        "{}{}",
        state.base_url.trim_end_matches('/'),
        OPENAI_CHAT_PATH
    );
    tracing::info!("Forwarding chat/completions request to {}", &backend_url);

    let response = state
        .client
        .post(&backend_url)
        .headers(fwd_headers)
        .body(body)
        .send()
        .await?;

    // 3. Pass through the response status and headers
    let status = axum::http::StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);

    let mut resp_headers = HeaderMap::new();
    for (key, value) in response.headers().iter() {
        if let Ok(name) = HeaderName::from_bytes(key.as_str().as_bytes()) {
            resp_headers.insert(name, value.clone());
        }
    }

    // 4. Check if this is a streaming response and pass through accordingly
    let is_stream = resp_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.contains("text/event-stream"))
        .unwrap_or(false);

    if is_stream {
        // Stream the response body directly to the client
        let byte_stream = response.bytes_stream();
        let body = axum::body::Body::from_stream(byte_stream);
        let mut resp = Response::builder().status(status).body(body).unwrap();
        *resp.headers_mut() = resp_headers;
        Ok(resp)
    } else {
        let body_bytes = response.bytes().await.unwrap_or_default();
        let mut resp = Response::builder()
            .status(status)
            .body(axum::body::Body::from(body_bytes))
            .unwrap();
        *resp.headers_mut() = resp_headers;
        Ok(resp)
    }
}

/// GET /health — proxy health check to the configured backend
pub async fn health_handler(State(state): State<AppState>) -> Result<Response, AppError> {
    let backend_url = format!("{}/health", state.base_url.trim_end_matches('/'));

    let response = state
        .client
        .get(&backend_url)
        .send()
        .await
        .map_err(|e| AppError::ApiError(format!("Health check failed: {}", e)))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    Ok((
        axum::http::StatusCode::from_u16(status.as_u16())
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::post};
    use serde_json::json;
    use tokio::net::TcpListener;

    async fn start_mock_backend() -> String {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|headers: HeaderMap, _body: String| async move {
                // Verify that app-key and app-sign headers are injected
                assert_eq!(headers.get("app-key").unwrap().to_str().unwrap(), "test-key");
                assert_eq!(headers.get("app-sign").unwrap().to_str().unwrap(), "test-sign");
                
                // We return a mock function calling response according to OpenAI docs
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

    #[tokio::test]
    async fn test_chat_completions_function_calling_proxy() {
        let backend_url = start_mock_backend().await;
        
        let config = Arc::new(AppConfig {
            app_key: "test-key".into(),
            app_sign: "test-sign".into(),
            openai_base_url: Some(backend_url.clone()),
            port: 0,
        });

        let state = AppState {
            config,
            client: Client::new(),
            base_url: backend_url,
        };

        let app = Router::new()
            .route("/v1/chat/completions", post(chat_completions_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let req_body = json!({
            "model": "gpt-4o",
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

        let res = client.post(format!("http://{}/v1/chat/completions", proxy_addr))
            .json(&req_body)
            .send()
            .await
            .expect("Request failed");

        assert_eq!(res.status(), 200);
        let res_json: serde_json::Value = res.json().await.unwrap();
        
        assert_eq!(res_json["choices"][0]["message"]["tool_calls"][0]["function"]["name"], "get_weather");
        assert_eq!(res_json["choices"][0]["finish_reason"], "tool_calls");
    }
}
