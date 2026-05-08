use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::response::{IntoResponse, Response, Sse};
use reqwest::Client;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::anthropic::AnthropicRequest;
use crate::config::{AppConfig, lookup_token};
use crate::error::AppError;
use crate::openai::OpenAISSEChunk;
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
    // 1. Extract Bearer token
    let token = extract_bearer_token(&headers)
        .ok_or_else(|| AppError::AuthenticationError("Missing Authorization header".into()))?;

    // 2. Lookup credentials
    let creds = lookup_token(&state.config, token)
        .ok_or_else(|| AppError::AuthenticationError("Invalid bearer token".into()))?;

    // 3. Parse Anthropic request
    let anthropic_req: AnthropicRequest = serde_json::from_str(&body)
        .map_err(|e| AppError::InvalidRequestError(format!("Invalid request body: {}", e)))?;

    let is_stream = anthropic_req.stream;

    // 4. Convert request
    let openai_req = convert_request(anthropic_req);

    // 5. Build forwarded headers: keep all except Authorization, add App-Key + App-Sign
    let mut fwd_headers = HeaderMap::new();
    for (key, value) in headers.iter() {
        if key.as_str().to_lowercase() != "authorization" {
            fwd_headers.insert(key, value.clone());
        }
    }
    fwd_headers.insert(
        HeaderName::from_static("app-key"),
        HeaderValue::from_str(&creds.app_key)
            .map_err(|_| AppError::ApiError("Invalid app_key value".into()))?,
    );
    fwd_headers.insert(
        HeaderName::from_static("app-sign"),
        HeaderValue::from_str(&creds.app_sign)
            .map_err(|_| AppError::ApiError("Invalid app_sign value".into()))?,
    );

    // 6. Send request to backend
    let backend_url = format!(
        "{}{}",
        state.base_url.trim_end_matches('/'),
        OPENAI_CHAT_PATH
    );
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

fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    let auth = headers.get("authorization")?.to_str().ok()?;
    if let Some(token) = auth.strip_prefix("Bearer ") {
        Some(token)
    } else {
        auth.strip_prefix("bearer ")
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

/// GET /health
pub async fn health_handler() -> &'static str {
    "ok"
}
