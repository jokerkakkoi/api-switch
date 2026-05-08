# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
# Build the project
cargo build

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

### Module Map

```
src/main.rs           → Entry point, route registration, app bootstrap
src/config.rs         → YAML config loading, token→credentials lookup, BASE_URL
src/error.rs          → AppError enum with Anthropic-format error responses
src/handler.rs        → /v1/messages + /health handlers, header forwarding logic
src/anthropic/mod.rs  → Anthropic protocol structs (request, response, SSE events)
src/openai/mod.rs     → OpenAI protocol structs (request, response, SSE chunks)
src/transform/
  ├── mod.rs          → Module re-exports
  ├── request.rs      → Anthropic→OpenAI request conversion
  ├── response.rs     → OpenAI→Anthropic response conversion + error mapping
  └── stream.rs       → SSE chunk→event conversion with state machine
```

### Request Flow

1. Client sends `POST /v1/messages` with `Authorization: Bearer <token>` + Anthropic JSON body
2. Handler extracts the bearer token, looks up credentials (`App-Key` + `App-Sign`) from config
3. Anthropic request is deserialized and converted to OpenAI format (`transform/request.rs`)
4. Headers are forwarded to backend, replacing `Authorization` with `App-Key` + `App-Sign`
5. Backend call to `{BASE_URL}/v1/chat/completions`
6. Response converted back: OpenAI → Anthropic (`transform/response.rs` for non-streaming, `transform/stream.rs` for SSE)

### Key Design Decisions

- **Config**: YAML file at path from `CONFIG_PATH` env var (falls back to `./config.yaml`). Maps Anthropic bearer tokens to backend credentials (`app_key` + `app_sign`).
- **Base URL**: From `BASE_URL` env var (falls back to `https://api.openai.com`).
- **Server binds to `0.0.0.0:3000`** (hardcoded, not configurable via config file).
- **Downstream client** has a 300-second timeout (`src/main.rs:20`).
- **Stop reason mapping**: OpenAI `stop` → Anthropic `end_turn`, OpenAI `length` → `max_tokens`.
- **`top_k`** is silently dropped (no OpenAI equivalent).
- **Anthropic `system` field** (string or content blocks) becomes a prepended `role: "system"` OpenAI message.
- **Image content**: Anthropic `{"type":"image","source":{...}}` → OpenAI `{"type":"image_url","image_url":{"url":"data:..."}}`.
- **Streaming** uses an `mpsc` channel (buffer 32) to bridge the spawned SSE parsing task and the axum SSE response stream. SSE frames are parsed by accumulating bytes and splitting on `\n\n`.
- **Error responses** always use Anthropic error format (`{"type":"error","error":{"type":"...","message":"..."}}`) regardless of which side produced the error.

### Runtime

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
  -d '{"model":"claude-3-opus","messages":[{"role":"user","content":"Hello"}],"max_tokens":1024}'
```
