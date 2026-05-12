# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
# Build the project
cargo build -r

# Run all tests (tests are inline in src/ modules)
cargo test

# Run tests for a specific module
cargo test -- config
cargo test -- error
cargo test -- anthropic
cargo test -- openai
cargo test -- transform::request
cargo test -- transform::response
cargo test -- transform::stream

# Run with verbose output
cargo test -- --nocapture
```

## Architecture

This is an **Anthropic → OpenAI protocol conversion gateway** — an axum HTTP server that accepts Anthropic Messages API requests, converts them to OpenAI Chat Completions API format, forwards to an OpenAI-compatible backend, and converts responses back to Anthropic format. Both streaming (SSE) and non-streaming paths are supported.

### Dependencies

Key crates:
- `axum` - HTTP server framework
- `tokio` - Async runtime
- `reqwest` - HTTP client for backend calls
- `serde`/`serde_json` - JSON serialization
- `serde_yaml` - Config file parsing
- `sse-stream` / `eventsource-stream` - SSE parsing

### Module Map

```
src/main.rs           → Entry point, route registration, app bootstrap, AxumState setup
src/config.rs         → YAML config loading, token→credentials lookup, BASE_URL, TokenConfig struct
src/error.rs          → AppError enum with Anthropic-format error responses (27 variants)
src/handler.rs        → /v1/messages + /health handlers, header forwarding, token extraction
src/anthropic/mod.rs  → Anthropic protocol structs (MessagesRequest, MessagesResponse, SSE events)
src/openai/mod.rs     → OpenAI protocol structs (ChatRequest, ChatResponse, SSE chunks)
src/transform/
  ├── mod.rs          → Module re-exports
  ├── request.rs      → Anthropic→OpenAI request conversion (transform_request fn)
  ├── response.rs     → OpenAI→Anthropic response conversion + error mapping
  └── stream.rs       → SSE chunk→event conversion with SseState state machine
```

### Configuration Structures

```rust
// Config loaded from YAML
struct Config {
    tokens: HashMap<String, TokenConfig>,
}

struct TokenConfig {
    app_key: String,      // Backend API key
    app_sign: String,     // App signature
}
```

### Request Flow

```
Client (Anthropic format)
      │
      ▼
POST /v1/messages
      │
      ├─ 1. Extract Bearer token from Authorization header
      ├─ 2. Look up credentials (App-Key, App-Sign) in config by token
      ├─ 3. Deserialize Anthropic request body
      ├─ 4. Transform Anthropic → OpenAI request (transform/request.rs)
      ├─ 5. Forward to {BASE_URL}/v1/chat/completions
      │      Headers: App-Key + App-Sign + original headers (minus Authorization)
      ├─ 6. Transform OpenAI response → Anthropic response
      │      - Non-streaming: transform/response.rs
      │      - Streaming: transform/stream.rs (SSE parsing)
      └─ 7. Return to client
```

### Key Design Decisions

- **Config**: YAML file at path from `CONFIG_PATH` env var (falls back to `./config.yaml`). Maps Anthropic bearer tokens to backend credentials (`app_key` + `app_sign`).
- **Base URL**: From `BASE_URL` env var (falls back to `https://api.openai.com`).
- **Server binds to `0.0.0.0:3000`** (hardcoded, not configurable via config file).
- **Downstream client** has a 300-second timeout (`src/main.rs:20`).
- **Stop reason mapping**: OpenAI `stop` → Anthropic `end_turn`, OpenAI `length` → `max_tokens`, OpenAI `tool_calls` → `tool_use`.
- **`top_k`** is silently dropped (no OpenAI equivalent).
- **Anthropic `system` field** (string or content blocks) becomes a prepended `role: "system"` OpenAI message.
- **Image content**: Anthropic `{"type":"image","source":{...}}` → OpenAI `{"type":"image_url","image_url":{"url":"data:..."}}`.
- **Streaming** uses an `mpsc` channel (buffer 32) to bridge the spawned SSE parsing task and the axum SSE response stream. SSE frames are parsed by accumulating bytes and splitting on `\n\n`.
- **Error responses** always use Anthropic error format (`{"type":"error","error":{"type":"...","message":"..."}}`) regardless of which side produced the error.
- **Header forwarding**: Original request headers (except `Authorization`, `Host`, `Content-Length`) are forwarded to backend with `App-Key` and `App-Sign` added.

### Runtime

```bash
export BASE_URL="https://your-backend.com"
export CONFIG_PATH="./config.yaml"
cargo run
```

Test with:
```bash
# Non-streaming request
curl -X POST http://localhost:3000/v1/messages \
  -H "Authorization: Bearer sk-ant-example-token" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model":"claude-sonnet-4-20250514","messages":[{"role":"user","content":"Hello"}],"max_tokens":1024}'

# Streaming request
curl -X POST http://localhost:3000/v1/messages \
  -H "Authorization: Bearer sk-ant-example-token" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model":"claude-sonnet-4-20250514","messages":[{"role":"user","content":"Hello"}],"max_tokens":1024,"stream":true}'

# Health check
curl http://localhost:3000/health
```
