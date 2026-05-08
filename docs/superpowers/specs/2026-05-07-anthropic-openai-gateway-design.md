# Anthropic → OpenAI Protocol Conversion Gateway Design

**Date**: 2026-05-07
**Status**: Approved

## Overview

An axum-based HTTP gateway that converts Anthropic Messages API requests to OpenAI Chat Completions API format, forwards them to an OpenAI-compatible backend, and converts responses back to Anthropic format.

## Architecture: Modular (Plan B)

```
src/
├── main.rs          # Entry point + route registration
├── config.rs        # YAML config loading + token → credentials lookup
├── error.rs         # Error types + Anthropic error response format
├── anthropic/
│   └── mod.rs       # Anthropic request/response/SSE event structs
├── openai/
│   └── mod.rs       # OpenAI request/response/SSE event structs
├── transform/
│   ├── mod.rs
│   ├── request.rs   # Anthropic → OpenAI request conversion
│   ├── response.rs  # OpenAI → Anthropic response conversion
│   └── stream.rs    # SSE streaming conversion
└── handler.rs       # axum handler + HTTP forwarding logic
```

## Routes

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/messages` | Anthropic Messages API → OpenAI conversion |
| `GET` | `/health` | Health check |

## Request Flow

```
Client request (Authorization: Bearer <token> + Anthropic body)
  │
  ▼
/v1/messages handler
  │
  ├─ 1. Extract Authorization: Bearer <token>
  ├─ 2. Lookup config: token → { App-Key, App-Sign }
  ├─ 3. Deserialize Anthropic request body
  ├─ 4. Check stream flag
  ├─ 5. Anthropic request → OpenAI request (transform)
  ├─ 6. Build HTTP request to {BASE_URL}/v1/chat/completions
  │     Headers: original - Authorization + App-Key + App-Sign
  ├─ 7. Send request (with reqwest)
  ├─ 8. OpenAI response → Anthropic response (transform)
  └─ 9. Return to client
```

## Config File Format (YAML)

```yaml
tokens:
  sk-ant-xxx:
    app_key: "your-openai-api-key"
    app_sign: "your-app-sign"
  sk-ant-yyy:
    app_key: "another-api-key"
    app_sign: "another-sign"
```

- Path: `CONFIG_PATH` env var, fallback `./config.yaml`

## Base URL

- From `BASE_URL` environment variable.

## Request Header Transformation

```
Original headers →                             Forwarded headers:
Authorization: Bearer sk-ant-xx                (removed)
Content-Type: application/json                 Content-Type: application/json
X-Custom: foo                                  X-Custom: foo
                                               App-Key: <from config>
                                               App-Sign: <from config>
```

All original headers pass through except `Authorization` which is replaced by `App-Key` + `App-Sign`.

## Protocol Conversion Mappings

### Request: Anthropic → OpenAI

| Anthropic field | OpenAI field | Notes |
|----------------|-------------|-------|
| `model` | `model` | Direct passthrough |
| `system` | `messages` prepend `role:"system"` | Field-level → role-based |
| `messages[].role` | `messages[].role` | Direct passthrough |
| `messages[].content` | `messages[].content` | See content mapping |
| `max_tokens` | `max_tokens` | Direct passthrough |
| `stop_sequences` | `stop` | Field rename |
| `temperature` | `temperature` | Direct passthrough |
| `top_p` | `top_p` | Direct passthrough |
| `top_k` | — | Dropped (no OpenAI equivalent) |
| `stream` | `stream` | Direct passthrough |

**Content mapping**:
- Text: `{"type":"text","text":"hi"}` → keep as-is
- Image: `{"type":"image","source":{...}}` → `{"type":"image_url","image_url":{"url":"..."}}`

### Response: OpenAI → Anthropic

| OpenAI field | Anthropic field | Notes |
|-------------|----------------|-------|
| `id` | `id` | Direct passthrough |
| `model` | `model` | Direct passthrough |
| `choices[0].message.role` | `role` | Fixed "assistant" |
| `choices[0].message.content` | `content: [{"type":"text","text":"..."}]` | String → content block |
| `choices[0].finish_reason` | `stop_reason` | Field rename |
| `usage` | `usage` | Direct passthrough |

### SSE Streaming: OpenAI deltas → Anthropic events

| Anthropic SSE event | Triggered by |
|---------------------|-------------|
| `message_start` | First delta (contains role) |
| `content_block_start` | First content delta |
| `content_block_delta` | Subsequent content deltas |
| `content_block_stop` | Text block end |
| `message_delta` | Final delta (contains finish_reason) |
| `message_stop` | Stream end |
| `ping` | Idle keepalive |

## Error Mapping

| Scenario | HTTP Status | Anthropic Error Type |
|----------|------------|---------------------|
| Token not found in config | 401 | `authentication_error` |
| Backend 401/403 | 401 | `authentication_error` |
| Backend 429 | 429 | `rate_limit_error` |
| Backend 5xx | 502 | `api_error` |
| Backend timeout | 504 | `timeout_error` |
| Invalid request body | 400 | `invalid_request_error` |
| Backend unreachable | 502 | `api_error` |

Error response format (Anthropic-style):
```json
{
  "type": "error",
  "error": {
    "type": "authentication_error",
    "message": "Invalid bearer token"
  }
}
```

## Dependencies

```toml
[dependencies]
axum = "0.8.9"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json", "stream"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
tracing = "0.1"
tracing-subscriber = "0.3"
anyhow = "1"
uuid = { version = "1", features = ["v4"] }
```
