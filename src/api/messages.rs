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
