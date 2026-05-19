# AGENTS.md

## Commands

```bash
cargo build          # build
cargo test           # all tests (inline in src/ modules)
cargo test -- <mod>  # single module, e.g. config, transform::stream
cargo test -- --nocapture  # show test stdout
cargo run            # requires config.yaml next to the binary
```

No lint, fmt, or clippy config is set up. Use `cargo clippy` and `cargo fmt` manually if needed.

## Config (IMPORTANT — CLAUDE.md is stale)

Config is a **multi-model routing** YAML. The real struct:

```yaml
models:
  - name: qwen35-397b
    app_key: "key1"
    app_sign: "sign1"
    base_url: http://host1/     # no trailing /v1/chat/completions
  - name: glm-5
    app_key: "key2"
    app_sign: "sign2"
    base_url: http://host2/
port: 3000                       # default, optional (falls back to 3000)
```

- Loaded from `CONFIG_PATH` env var, fallback `./config.yaml`
- `config.yaml` is gitignored — copy `config.yaml.example` to start
- Port comes from config, NOT hardcoded
- Base URL comes from `base_url` field per model, NOT from env var
- Model name in incoming request determines which model config to use via `AppConfig::find_model()`
- Unknown model names return 400 `InvalidRequestError`

## Routes

| Method | Path | Handler |
|--------|------|---------|
| POST | `/v1/messages` | Anthropic → OpenAI proxy (protocol conversion) |
| POST | `/v1/chat/completions` | OpenAI passthrough (transparent proxy, no conversion) |
| GET | `/health` | Health check (proxies to first model's backend `/health`) |

## Architecture

Anthropic → OpenAI protocol conversion gateway (axum HTTP server).

```text
src/main.rs           → Entry, route registration, port from config, 300s client timeout
src/config.rs         → Multi-model AppConfig (Vec<ModelConfig>, port), find_model() lookup
src/error.rs          → AppError enum (5 variants), Anthropic-format error responses
src/api/
  ├── mod.rs          → API module entry, route handler exports
  ├── messages.rs     → Anthropic → OpenAI proxy (protocol conversion)
  ├── chat_completions.rs → OpenAI passthrough (transparent proxy)
  └── health.rs       → Health check (proxies to first model's backend)
src/anthropic/mod.rs  → Anthropic protocol types (request, response, SSE events, tools)
src/openai/mod.rs     → OpenAI protocol types (request, response, SSE chunks, tools)
src/transform/
  ├── request.rs      → Anthropic → OpenAI request conversion (tools, images, tool_results)
  ├── response.rs     → OpenAI → Anthropic response conversion, error mapping
  ├── stream.rs       → SSE state machine (OpenAI chunk → Anthropic events, tool streaming)
  └── headers.rs      → Header forwarding logic (skip headers, app-key/app-sign injection)
```

## Key behaviors

- Downstream HTTP client timeout: 300s, no_proxy
- `top_k` silently dropped (no OpenAI equivalent)
- Anthropic `system` field → prepended `role: "system"` message (string or content list)
- Stop reason mapping: `stop`→`end_turn`, `length`→`max_tokens`, `tool_calls`→`tool_use`, `content_filter`→`content_filter`
- All errors use Anthropic error format (`{ "type": "error", "error": { "type": "...", "message": "..." } }`)
- SSE streaming: mpsc channel (buffer 32), split on `\n\n`, state machine tracks content blocks and tool calls
- Tool calling: full support for Anthropic ↔ OpenAI tool format conversion, streaming tool call accumulation
- Image content: Anthropic base64 images → OpenAI data URL format
- Tool results: Anthropic `tool_result` blocks → separate OpenAI `role: "tool"` messages
- Extra fields (`#[serde(flatten)]`) forwarded through both request and response
- OpenAI-style tools (`{ "type": "function", "function": {...} }`) accepted alongside native Anthropic format
- Request/response body logged on backend non-200 errors via `tracing::error!`
- Platform: Windows (`.exe` in gitignore, release builds produce `.exe`)
- Edition: Rust 2024
