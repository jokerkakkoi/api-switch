# Anthropic → OpenAI Protocol Conversion Gateway Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an axum-based HTTP gateway that converts Anthropic Messages API requests into OpenAI Chat Completions API, forwards them, and converts responses back.

**Architecture:** Modular design with 8 source files — config loading, error types, Anthropic/OpenAI data models, three transform modules (request/response/stream), handler, and main entry point. Uses reqwest for HTTP forwarding, channels for SSE streaming.

**Tech Stack:** Rust 2024 edition, axum 0.8.9, reqwest 0.12, serde_yaml, tokio, tokio-stream, uuid

**File Structure:**
```
src/
├── main.rs              # Entry point, route registration, app setup
├── config.rs            # YAML config loading, token→credentials lookup, base_url
├── error.rs             # AppError enum, Anthropic-format error responses
├── anthropic/
│   └── mod.rs           # Anthropic request/response/SSE event structs
├── openai/
│   └── mod.rs           # OpenAI request/response/SSE chunk structs
├── transform/
│   ├── mod.rs           # Re-exports
│   ├── request.rs       # Anthropic → OpenAI request conversion
│   ├── response.rs      # OpenAI → Anthropic response conversion
│   └── stream.rs        # SSE streaming event conversion + formatting
└── handler.rs           # /v1/messages handler, header forwarding, HTTP dispatch
```

---

### Task 1: Update Dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Replace Cargo.toml with full dependencies**

```toml
[package]
name = "rust-project"
version = "0.1.0"
edition = "2024"

[dependencies]
axum = "0.8.9"
tokio = { version = "1", features = ["full"] }
tokio-stream = "0.1"
reqwest = { version = "0.12", features = ["json", "stream"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
tracing = "0.1"
tracing-subscriber = "0.3"
anyhow = "1"
uuid = { version = "1", features = ["v4"] }
futures = "0.3"
```

- [ ] **Step 2: Build to verify dependencies resolve**

Run: `cargo build`
Expected: Dependencies download and compile. `src/main.rs` still has hello-world but compiles.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: add all project dependencies"
```

---

### Task 2: Config Module

**Files:**
- Create: `src/config.rs`

- [ ] **Step 1: Write config.rs**

```rust
use serde::Deserialize;
use std::collections::HashMap;
use std::env;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub tokens: HashMap<String, TokenCredentials>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TokenCredentials {
    pub app_key: String,
    pub app_sign: String,
}

/// Load config from CONFIG_PATH env var, fallback to ./config.yaml
pub fn load_config() -> AppConfig {
    let path = env::var("CONFIG_PATH").unwrap_or_else(|_| "./config.yaml".to_string());
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read config file '{}': {}", path, e));
    serde_yaml::from_str(&contents)
        .unwrap_or_else(|e| panic!("Failed to parse config file '{}': {}", path, e))
}

/// Look up a bearer token in the config, returning credentials if found.
pub fn lookup_token<'a>(config: &'a AppConfig, token: &str) -> Option<&'a TokenCredentials> {
    config.tokens.get(token)
}

/// Get base URL from BASE_URL env var, fallback to OpenAI default.
pub fn base_url() -> String {
    env::var("BASE_URL").unwrap_or_else(|_| "https://api.openai.com".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_token_found() {
        let mut tokens = HashMap::new();
        tokens.insert(
            "sk-ant-xxx".to_string(),
            TokenCredentials {
                app_key: "key-123".to_string(),
                app_sign: "sign-abc".to_string(),
            },
        );
        let config = AppConfig { tokens };
        let result = lookup_token(&config, "sk-ant-xxx");
        assert!(result.is_some());
        assert_eq!(result.unwrap().app_key, "key-123");
    }

    #[test]
    fn test_lookup_token_not_found() {
        let config = AppConfig {
            tokens: HashMap::new(),
        };
        assert!(lookup_token(&config, "unknown").is_none());
    }

    #[test]
    fn test_base_url_default() {
        // Don't assert exact URL when BASE_URL may be set in env,
        // just verify it returns a non-empty string
        let url = base_url();
        assert!(!url.is_empty());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -- config`
Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/config.rs
git commit -m "feat: add config module with YAML loading and token lookup"
```

---

### Task 3: Error Module

**Files:**
- Create: `src/error.rs`

- [ ] **Step 1: Write error.rs**

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

#[derive(Debug)]
pub enum AppError {
    AuthenticationError(String),
    InvalidRequestError(String),
    RateLimitError(String),
    ApiError(String),
    TimeoutError(String),
}

#[derive(Serialize)]
struct AnthropicErrorResponse {
    #[serde(rename = "type")]
    error_type: String,
    error: AnthropicErrorDetail,
}

#[derive(Serialize)]
struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match &self {
            AppError::AuthenticationError(msg) => {
                (StatusCode::UNAUTHORIZED, "authentication_error", msg.clone())
            }
            AppError::InvalidRequestError(msg) => {
                (StatusCode::BAD_REQUEST, "invalid_request_error", msg.clone())
            }
            AppError::RateLimitError(msg) => {
                (StatusCode::TOO_MANY_REQUESTS, "rate_limit_error", msg.clone())
            }
            AppError::ApiError(msg) => {
                (StatusCode::BAD_GATEWAY, "api_error", msg.clone())
            }
            AppError::TimeoutError(msg) => {
                (StatusCode::GATEWAY_TIMEOUT, "timeout_error", msg.clone())
            }
        };

        let body = AnthropicErrorResponse {
            error_type: "error".to_string(),
            error: AnthropicErrorDetail {
                error_type,
                message,
            },
        };

        (status, Json(body)).into_response()
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            AppError::TimeoutError("Request to backend timed out".to_string())
        } else if err.is_connect() {
            AppError::ApiError("Failed to connect to backend".to_string())
        } else {
            AppError::ApiError(format!("Backend request failed: {}", err))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_error_returns_401() {
        let response = AppError::AuthenticationError("bad token".into()).into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_rate_limit_error_returns_429() {
        let response = AppError::RateLimitError("too many".into()).into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -- error`
Expected: 2 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/error.rs
git commit -m "feat: add error types with Anthropic-format error responses"
```

---

### Task 4: Anthropic Data Models

**Files:**
- Create: `src/anthropic/mod.rs`

- [ ] **Step 1: Write anthropic/mod.rs**

```rust
use serde::{Deserialize, Serialize};

/// Incoming Anthropic Messages API request body.
#[derive(Debug, Deserialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub system: Option<AnthropicSystem>,
    pub max_tokens: u32,
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub top_k: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: AnthropicContent,
}

/// Anthropic content can be a plain text string or an array of content blocks.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AnthropicContent {
    TextBlocks(Vec<AnthropicContentBlock>),
    SingleString(String),
}

#[derive(Debug, Deserialize, Clone)]
pub struct AnthropicContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub source: Option<ImageSource>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

/// system can be a plain string or a list of content blocks.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AnthropicSystem {
    String(String),
    ContentList(Vec<AnthropicContentBlock>),
}

/// Outgoing Anthropic response (non-streaming).
#[derive(Debug, Serialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: String,
    pub content: Vec<AnthropicResponseContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: AnthropicUsage,
}

#[derive(Debug, Serialize)]
pub struct AnthropicResponseContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct AnthropicUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Anthropic SSE event types for streaming.
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type")]
pub enum AnthropicSSEEvent {
    #[serde(rename = "message_start")]
    MessageStart {
        message: AnthropicStreamMessage,
    },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: u32,
        content_block: AnthropicResponseContentBlock,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: u32,
        delta: ContentDelta,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop {
        index: u32,
    },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDeltaData,
        usage: OutputUsage,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
}

#[derive(Debug, Serialize, Clone)]
pub struct AnthropicStreamMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub role: String,
    pub content: Vec<serde_json::Value>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: AnthropicUsage,
}

#[derive(Debug, Serialize, Clone)]
pub struct ContentDelta {
    #[serde(rename = "type")]
    pub delta_type: String,
    pub text: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct MessageDeltaData {
    pub stop_reason: String,
    pub stop_sequence: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct OutputUsage {
    pub output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_anthropic_request_with_string_content() {
        let json = r#"{
            "model": "claude-3-opus",
            "messages": [{"role": "user", "content": "Hello"}],
            "max_tokens": 1024
        }"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "claude-3-opus");
        assert_eq!(req.max_tokens, 1024);
        assert!(!req.stream);
    }

    #[test]
    fn test_deserialize_anthropic_request_with_stream() {
        let json = r#"{
            "model": "claude-3",
            "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}],
            "max_tokens": 500,
            "stream": true
        }"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        assert!(req.stream);
    }

    #[test]
    fn test_deserialize_system_as_string() {
        let json = r#"{
            "model": "claude-3",
            "messages": [{"role": "user", "content": "hi"}],
            "system": "You are helpful.",
            "max_tokens": 100
        }"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        match req.system.unwrap() {
            AnthropicSystem::String(s) => assert_eq!(s, "You are helpful."),
            _ => panic!("expected string system"),
        }
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -- anthropic`
Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/anthropic/
git commit -m "feat: add Anthropic protocol data models"
```

---

### Task 5: OpenAI Data Models

**Files:**
- Create: `src/openai/mod.rs`

- [ ] **Step 1: Write openai/mod.rs**

```rust
use serde::{Deserialize, Serialize};

/// Request body sent to OpenAI-compatible backend.
#[derive(Debug, Serialize)]
pub struct OpenAIRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    pub stream: bool,
}

#[derive(Debug, Serialize)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: OpenAIContent,
}

/// String for simple text, Vec for multi-content (text+image).
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum OpenAIContent {
    Text(String),
    MultiContent(Vec<OpenAIContentBlock>),
}

#[derive(Debug, Serialize)]
pub struct OpenAIContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<ImageUrl>,
}

#[derive(Debug, Serialize)]
pub struct ImageUrl {
    pub url: String,
}

/// Non-streaming response from OpenAI backend.
#[derive(Debug, Deserialize)]
pub struct OpenAIResponse {
    pub id: String,
    pub model: String,
    pub choices: Vec<OpenAIChoice>,
    pub usage: Option<OpenAIUsage>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIChoice {
    pub index: u32,
    pub message: OpenAIResponseMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIResponseMessage {
    pub role: String,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Single SSE chunk from OpenAI streaming response.
#[derive(Debug, Deserialize)]
pub struct OpenAISSEChunk {
    pub id: Option<String>,
    pub model: Option<String>,
    pub choices: Option<Vec<OpenAIDeltaChoice>>,
    pub usage: Option<OpenAIUsage>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIDeltaChoice {
    pub index: u32,
    pub delta: OpenAIDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIDelta {
    pub role: Option<String>,
    pub content: Option<String>,
}

/// Error response from OpenAI backend.
#[derive(Debug, Deserialize)]
pub struct OpenAIErrorResponse {
    pub error: OpenAIErrorDetail,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_openai_request() {
        let req = OpenAIRequest {
            model: "gpt-4".into(),
            messages: vec![OpenAIMessage {
                role: "user".into(),
                content: OpenAIContent::Text("hi".into()),
            }],
            max_tokens: Some(100),
            stop: None,
            temperature: None,
            top_p: None,
            stream: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"model\":\"gpt-4\""));
        assert!(json.contains("\"stream\":false"));
    }

    #[test]
    fn test_deserialize_openai_response() {
        let json = r#"{
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello!"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        }"#;
        let resp: OpenAIResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "chatcmpl-123");
        assert_eq!(resp.choices[0].message.content.as_ref().unwrap(), "Hello!");
    }

    #[test]
    fn test_deserialize_sse_chunk_with_delta() {
        let json = r#"{
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{"index": 0, "delta": {"content": "Hi"}, "finish_reason": null}]
        }"#;
        let chunk: OpenAISSEChunk = serde_json::from_str(json).unwrap();
        let choices = chunk.choices.unwrap();
        assert_eq!(choices[0].delta.content.as_ref().unwrap(), "Hi");
    }
}
```

- [ ] **Step 2: Verify tests pass**

Run: `cargo test -- openai`
Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/openai/
git commit -m "feat: add OpenAI protocol data models"
```

---

### Task 6: Request Transform

**Files:**
- Create: `src/transform/mod.rs`
- Create: `src/transform/request.rs`

- [ ] **Step 1: Write transform/mod.rs**

```rust
pub mod request;
pub mod response;
pub mod stream;
```

- [ ] **Step 2: Write transform/request.rs**

```rust
use crate::anthropic::{
    AnthropicContent, AnthropicContentBlock, AnthropicRequest, AnthropicSystem,
};
use crate::openai::{ImageUrl, OpenAIContent, OpenAIContentBlock, OpenAIMessage, OpenAIRequest};

pub fn convert_request(req: AnthropicRequest) -> OpenAIRequest {
    let mut messages = Vec::new();

    // Anthropic system field → OpenAI system-role message
    if let Some(system) = req.system {
        let system_text = match system {
            AnthropicSystem::String(s) => s,
            AnthropicSystem::ContentList(blocks) => blocks
                .into_iter()
                .filter_map(|b| b.text)
                .collect::<Vec<_>>()
                .join("\n\n"),
        };
        messages.push(OpenAIMessage {
            role: "system".to_string(),
            content: OpenAIContent::Text(system_text),
        });
    }

    // Convert messages
    for msg in req.messages {
        let content = convert_content(msg.content);
        messages.push(OpenAIMessage {
            role: msg.role,
            content,
        });
    }

    OpenAIRequest {
        model: req.model,
        messages,
        max_tokens: Some(req.max_tokens),
        stop: req.stop_sequences,
        temperature: req.temperature,
        top_p: req.top_p,
        stream: req.stream,
    }
}

fn convert_content(content: AnthropicContent) -> OpenAIContent {
    match content {
        AnthropicContent::SingleString(text) => OpenAIContent::Text(text),
        AnthropicContent::TextBlocks(blocks) => {
            // If single text block, use simple Text variant
            if blocks.len() == 1 && blocks[0].content_type == "text" {
                if let Some(ref text) = blocks[0].text {
                    return OpenAIContent::Text(text.clone());
                }
            }
            // Multi-block or image → convert each block
            let converted: Vec<OpenAIContentBlock> = blocks
                .into_iter()
                .map(convert_content_block)
                .collect();
            OpenAIContent::MultiContent(converted)
        }
    }
}

fn convert_content_block(block: AnthropicContentBlock) -> OpenAIContentBlock {
    match block.content_type.as_str() {
        "image" => {
            let url = block
                .source
                .map(|src| format!("data:{};base64,{}", src.media_type, src.data))
                .unwrap_or_default();
            OpenAIContentBlock {
                content_type: "image_url".to_string(),
                text: None,
                image_url: Some(ImageUrl { url }),
            }
        }
        _ => OpenAIContentBlock {
            content_type: "text".to_string(),
            text: block.text,
            image_url: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::{AnthropicMessage, AnthropicSystem};

    fn make_request(stream: bool) -> AnthropicRequest {
        AnthropicRequest {
            model: "claude-3-opus".into(),
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: AnthropicContent::SingleString("Hello!".into()),
            }],
            system: Some(AnthropicSystem::String("You are helpful.".into())),
            max_tokens: 1024,
            stop_sequences: None,
            stream,
            temperature: Some(0.7),
            top_p: None,
            top_k: Some(5),
        }
    }

    #[test]
    fn test_convert_basic_request() {
        let result = convert_request(make_request(false));
        assert_eq!(result.model, "claude-3-opus");
        assert_eq!(result.messages.len(), 2); // system + user
        assert_eq!(result.messages[0].role, "system");
        match &result.messages[0].content {
            OpenAIContent::Text(t) => assert_eq!(t, "You are helpful."),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_convert_stop_sequences() {
        let mut req = make_request(false);
        req.stop_sequences = Some(vec!["END".into(), "STOP".into()]);
        let result = convert_request(req);
        assert_eq!(result.stop, Some(vec!["END".into(), "STOP".into()]));
    }

    #[test]
    fn test_convert_stream_flag() {
        let result = convert_request(make_request(true));
        assert!(result.stream);
    }

    #[test]
    fn test_top_k_is_dropped() {
        let mut req = make_request(false);
        req.top_k = Some(10);
        let result = convert_request(req);
        // top_k has no OpenAI equivalent — just verify conversion succeeds
        assert_eq!(result.model, "claude-3-opus");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -- transform::request`
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/transform/
git commit -m "feat: add Anthropic→OpenAI request conversion"
```

---

### Task 7: Response Transform

**Files:**
- Create: `src/transform/response.rs`

- [ ] **Step 1: Write transform/response.rs**

```rust
use crate::anthropic::{AnthropicResponse, AnthropicResponseContentBlock, AnthropicUsage};
use crate::error::AppError;
use crate::openai::{OpenAIErrorResponse, OpenAIResponse};

/// Convert an OpenAI non-streaming response into an Anthropic response.
pub fn convert_response(resp: OpenAIResponse) -> AnthropicResponse {
    let choice = &resp.choices[0];
    let content = match &choice.message.content {
        Some(text) => vec![AnthropicResponseContentBlock {
            content_type: "text".to_string(),
            text: text.clone(),
        }],
        None => vec![],
    };

    let usage = match &resp.usage {
        Some(u) => AnthropicUsage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
        },
        None => AnthropicUsage {
            input_tokens: 0,
            output_tokens: 0,
        },
    };

    AnthropicResponse {
        id: resp.id,
        response_type: "message".to_string(),
        role: choice.message.role.clone(),
        content,
        model: resp.model,
        stop_reason: choice.finish_reason.clone(),
        stop_sequence: None,
        usage,
    }
}

/// Parse an OpenAI error body and map it to our AppError type.
pub fn map_openai_error(status: u16, body: &str) -> AppError {
    if let Ok(err_resp) = serde_json::from_str::<OpenAIErrorResponse>(body) {
        let msg = err_resp.error.message;
        match status {
            401 | 403 => AppError::AuthenticationError(msg),
            429 => AppError::RateLimitError(msg),
            _ => AppError::ApiError(msg),
        }
    } else {
        match status {
            401 | 403 => AppError::AuthenticationError(format!("Backend returned {}", status)),
            429 => AppError::RateLimitError(format!("Backend returned {}", status)),
            _ => AppError::ApiError(format!("Backend returned {}: {}", status, body)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{OpenAIChoice, OpenAIResponseMessage, OpenAIUsage};

    fn make_response(content: &str) -> OpenAIResponse {
        OpenAIResponse {
            id: "chatcmpl-123".into(),
            model: "gpt-4".into(),
            choices: vec![OpenAIChoice {
                index: 0,
                message: OpenAIResponseMessage {
                    role: "assistant".into(),
                    content: Some(content.into()),
                },
                finish_reason: Some("stop".into()),
            }],
            usage: Some(OpenAIUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            }),
        }
    }

    #[test]
    fn test_convert_response_content() {
        let result = convert_response(make_response("Hello!"));
        assert_eq!(result.id, "chatcmpl-123");
        assert_eq!(result.role, "assistant");
        assert_eq!(result.content[0].content_type, "text");
        assert_eq!(result.content[0].text, "Hello!");
    }

    #[test]
    fn test_convert_response_usage() {
        let result = convert_response(make_response("Hi"));
        assert_eq!(result.usage.input_tokens, 10);
        assert_eq!(result.usage.output_tokens, 5);
    }

    #[test]
    fn test_convert_response_stop_reason() {
        let result = convert_response(make_response("ok"));
        assert_eq!(result.stop_reason, Some("stop".into()));
    }

    #[test]
    fn test_map_openai_error_auth() {
        let body = r#"{"error":{"message":"Bad API key","type":"invalid_request_error"}}"#;
        let err = map_openai_error(401, body);
        match err {
            AppError::AuthenticationError(msg) => assert!(msg.contains("Bad API key")),
            _ => panic!("expected AuthenticationError"),
        }
    }

    #[test]
    fn test_map_openai_error_rate_limit() {
        let body = r#"{"error":{"message":"Rate limited","type":"rate_limit"}}"#;
        let err = map_openai_error(429, body);
        match err {
            AppError::RateLimitError(msg) => assert!(msg.contains("Rate limited")),
            _ => panic!("expected RateLimitError"),
        }
    }

    #[test]
    fn test_map_openai_error_server_error() {
        let err = map_openai_error(500, "Internal error");
        match err {
            AppError::ApiError(_) => {}
            _ => panic!("expected ApiError"),
        }
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -- transform::response`
Expected: 6 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/transform/response.rs
git commit -m "feat: add OpenAI→Anthropic response conversion"
```

---

### Task 8: Stream Transform (SSE)

**Files:**
- Create: `src/transform/stream.rs`

- [ ] **Step 1: Write transform/stream.rs**

```rust
use crate::anthropic::{
    AnthropicResponseContentBlock, AnthropicSSEEvent, AnthropicStreamMessage, AnthropicUsage,
    ContentDelta, MessageDeltaData, OutputUsage,
};
use crate::openai::OpenAISSEChunk;
use uuid::Uuid;

/// Tracks state across SSE chunks to produce the correct Anthropic event sequence.
pub struct StreamState {
    pub message_id: String,
    pub model: String,
    pub input_tokens: u32,
    pub content_index: u32,
    pub started: bool,
    pub content_block_open: bool,
}

impl StreamState {
    pub fn new() -> Self {
        Self {
            message_id: format!("msg_{}", &Uuid::new_v4().to_string().replace('-', "")[..24]),
            model: String::new(),
            input_tokens: 0,
            content_index: 0,
            started: false,
            content_block_open: false,
        }
    }
}

/// Convert a single OpenAI SSE chunk into zero or more Anthropic SSE events.
pub fn convert_stream_chunk(
    chunk: &OpenAISSEChunk,
    state: &mut StreamState,
) -> Vec<AnthropicSSEEvent> {
    let mut events = Vec::new();

    // Capture model from first chunk
    if let Some(ref model) = chunk.model {
        if state.model.is_empty() {
            state.model = model.clone();
        }
    }

    // Capture prompt token count
    if let Some(ref usage) = chunk.usage {
        state.input_tokens = usage.prompt_tokens;
    }

    // On first chunk with data, emit message_start
    if !state.started {
        events.push(AnthropicSSEEvent::MessageStart {
            message: AnthropicStreamMessage {
                id: state.message_id.clone(),
                msg_type: "message".to_string(),
                role: "assistant".to_string(),
                content: vec![],
                model: state.model.clone(),
                stop_reason: None,
                stop_sequence: None,
                usage: AnthropicUsage {
                    input_tokens: state.input_tokens,
                    output_tokens: 0,
                },
            },
        });
        state.started = true;
    }

    // Process choice deltas
    if let Some(ref choices) = chunk.choices {
        for choice in choices {
            // Role delta → start content block
            if choice.delta.role.is_some() && !state.content_block_open {
                events.push(AnthropicSSEEvent::ContentBlockStart {
                    index: state.content_index,
                    content_block: AnthropicResponseContentBlock {
                        content_type: "text".to_string(),
                        text: String::new(),
                    },
                });
                state.content_block_open = true;
            }

            // Content delta
            if let Some(ref text) = choice.delta.content {
                if !state.content_block_open {
                    events.push(AnthropicSSEEvent::ContentBlockStart {
                        index: state.content_index,
                        content_block: AnthropicResponseContentBlock {
                            content_type: "text".to_string(),
                            text: String::new(),
                        },
                    });
                    state.content_block_open = true;
                }
                events.push(AnthropicSSEEvent::ContentBlockDelta {
                    index: state.content_index,
                    delta: ContentDelta {
                        delta_type: "text_delta".to_string(),
                        text: text.clone(),
                    },
                });
            }

            // Finish reason → end content block + message stop
            if let Some(ref finish_reason) = choice.finish_reason {
                if state.content_block_open {
                    events.push(AnthropicSSEEvent::ContentBlockStop {
                        index: state.content_index,
                    });
                    state.content_block_open = false;
                }

                let stop_reason = map_stop_reason(finish_reason);
                let output_tokens =
                    chunk.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0);

                events.push(AnthropicSSEEvent::MessageDelta {
                    delta: MessageDeltaData {
                        stop_reason,
                        stop_sequence: None,
                    },
                    usage: OutputUsage { output_tokens },
                });

                events.push(AnthropicSSEEvent::MessageStop);
            }
        }
    }

    events
}

fn map_stop_reason(reason: &str) -> String {
    match reason {
        "stop" => "end_turn".to_string(),
        "length" => "max_tokens".to_string(),
        "content_filter" => "content_filter".to_string(),
        _ => reason.to_string(),
    }
}

/// Format an Anthropic SSE event into SSE wire format.
pub fn format_sse(event: &AnthropicSSEEvent) -> String {
    let event_type = match event {
        AnthropicSSEEvent::MessageStart { .. } => "message_start",
        AnthropicSSEEvent::ContentBlockStart { .. } => "content_block_start",
        AnthropicSSEEvent::ContentBlockDelta { .. } => "content_block_delta",
        AnthropicSSEEvent::ContentBlockStop { .. } => "content_block_stop",
        AnthropicSSEEvent::MessageDelta { .. } => "message_delta",
        AnthropicSSEEvent::MessageStop => "message_stop",
        AnthropicSSEEvent::Ping => "ping",
    };
    let data = serde_json::to_string(event).unwrap();
    format!("event: {}\ndata: {}\n\n", event_type, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{OpenAIDelta, OpenAIDeltaChoice, OpenAIUsage};

    #[test]
    fn test_stream_state_new() {
        let state = StreamState::new();
        assert!(!state.started);
        assert!(!state.content_block_open);
        assert_eq!(state.content_index, 0);
    }

    #[test]
    fn test_first_chunk_with_role_emits_start_and_block_start() {
        let chunk = make_chunk(Some("assistant"), None, None, None, None);
        let mut state = StreamState::new();
        let events = convert_stream_chunk(&chunk, &mut state);

        assert!(state.started);
        assert!(state.content_block_open);
        let has_msg_start = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::MessageStart { .. }));
        let has_block_start = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockStart { .. }));
        assert!(has_msg_start);
        assert!(has_block_start);
    }

    #[test]
    fn test_content_delta_emits_delta_event() {
        let chunk = make_chunk(None, Some("Hello"), None, None, None);
        let mut state = StreamState::new();
        let events = convert_stream_chunk(&chunk, &mut state);

        let has_delta = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockDelta { .. }));
        assert!(has_delta);
    }

    #[test]
    fn test_finish_reason_emits_stop_events() {
        let usage = OpenAIUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        };
        let chunk = make_chunk(None, None, Some("stop"), Some("gpt-4"), Some(usage));
        let mut state = StreamState::new();
        state.started = true;
        let events = convert_stream_chunk(&chunk, &mut state);

        // Should NOT have content_block_start since we passed content=None and role=None
        let has_block_stop = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockStop { .. }));
        let has_msg_delta = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::MessageDelta { .. }));
        let has_msg_stop = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::MessageStop));
        // Since content_block_open was false and no content arrived, block_stop won't emit
        assert!(has_msg_delta);
        assert!(has_msg_stop);
    }

    #[test]
    fn test_format_sse_ping() {
        let event = AnthropicSSEEvent::Ping;
        let output = format_sse(&event);
        assert!(output.starts_with("event: ping\n"));
        assert!(output.ends_with("\n\n"));
    }

    #[test]
    fn test_map_stop_reason_mappings() {
        assert_eq!(map_stop_reason("stop"), "end_turn");
        assert_eq!(map_stop_reason("length"), "max_tokens");
    }

    // Helper
    fn make_chunk(
        role: Option<&str>,
        content: Option<&str>,
        finish_reason: Option<&str>,
        model: Option<&str>,
        usage: Option<OpenAIUsage>,
    ) -> OpenAISSEChunk {
        let has_choice_data =
            role.is_some() || content.is_some() || finish_reason.is_some();
        OpenAISSEChunk {
            id: Some("chatcmpl-123".into()),
            model: model.map(String::from),
            choices: if has_choice_data {
                Some(vec![OpenAIDeltaChoice {
                    index: 0,
                    delta: OpenAIDelta {
                        role: role.map(String::from),
                        content: content.map(String::from),
                    },
                    finish_reason: finish_reason.map(String::from),
                }])
            } else {
                None
            },
            usage,
        }
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -- transform::stream`
Expected: 6 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/transform/stream.rs
git commit -m "feat: add SSE streaming protocol conversion"
```

---

### Task 9: Handler

**Files:**
- Create: `src/handler.rs`

- [ ] **Step 1: Write handler.rs**

```rust
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::response::{IntoResponse, Response, Sse};
use axum::Json;
use reqwest::Client;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use crate::anthropic::AnthropicRequest;
use crate::config::{lookup_token, AppConfig};
use crate::error::AppError;
use crate::openai::OpenAISSEChunk;
use crate::transform::request::convert_request;
use crate::transform::response::{convert_response, map_openai_error};
use crate::transform::stream::{convert_stream_chunk, format_sse, StreamState};

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
    let backend_url = format!("{}{}", state.base_url.trim_end_matches('/'), OPENAI_CHAT_PATH);
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
async fn handle_stream_response(
    request: reqwest::RequestBuilder,
) -> Result<Response, AppError> {
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
                            if let Ok(chunk) =
                                serde_json::from_str::<OpenAISSEChunk>(data_str)
                            {
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
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles successfully. (May have an unused import warning for `Infallible` if unused — this is OK, will be cleaned up.)

- [ ] **Step 3: Commit**

```bash
git add src/handler.rs
git commit -m "feat: add /v1/messages handler with header forwarding and streaming"
```

---

### Task 10: Main Entry Point

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Write main.rs**

```rust
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
```

- [ ] **Step 2: Build the complete project**

Run: `cargo build`
Expected: Full compile succeeds.

- [ ] **Step 3: Run all unit tests**

Run: `cargo test`
Expected: All tests pass (config: 3, error: 2, anthropic: 3, openai: 3, request: 4, response: 6, stream: 6 = 27 tests).

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire up main entry point with routes and state"
```

---

### Task 11: Create Example Config and Verify

**Files:**
- Create: `config.yaml.example`

- [ ] **Step 1: Write config.yaml.example**

```yaml
tokens:
  sk-ant-example-token:
    app_key: "sk-your-openai-api-key"
    app_sign: "your-app-sign-value"
```

- [ ] **Step 2: Final verification — check everything compiles and tests pass**

Run: `cargo build && cargo test`
Expected: Build succeeds, all 27 tests pass.

- [ ] **Step 3: Verify error format is correct**

Run: `cargo test -- error --nocapture`
Expected: Error module tests pass.

- [ ] **Step 4: Commit**

```bash
git add config.yaml.example .gitignore
git commit -m "docs: add example config file"
```

---

## Summary

**Total files created:** 10 source files + 1 example config
**Total unit tests:** 27
**Key dependencies:** axum 0.8.9, reqwest 0.12, serde_yaml 0.9, tokio 1, uuid 1

**After all tasks, run to start:**
```bash
export BASE_URL="https://your-backend.com"
export CONFIG_PATH="./config.yaml"
cargo run
```

Test with:
```bash
curl -X POST http://localhost:3000/v1/messages \
  -H "Authorization: Bearer sk-ant-example-token" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-3-opus",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 1024
  }'
```
