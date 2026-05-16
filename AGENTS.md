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

Config is a **flat single-tenant** YAML, NOT a token HashMap. The real struct:

```yaml
app_key: "..."
app_sign: "..."
openai_base_url: http://host:port/path/   # no trailing /v1/chat/completions
port: 3000                                 # default, configurable
```

- Loaded from `CONFIG_PATH` env var, fallback `./config.yaml`
- `config.yaml` is gitignored — copy `config.yaml.example` to start
- Port comes from config, NOT hardcoded
- Base URL comes from `openai_base_url` field, NOT from `BASE_URL` env var

## Routes

| Method | Path | Handler |
|--------|------|---------|
| POST | `/v1/messages` | Anthropic → OpenAI proxy |
| POST | `/v1/chat/completions` | Direct OpenAI passthrough |
| GET | `/health` | Health check |

## Architecture

Anthropic → OpenAI protocol conversion gateway (axum HTTP server).

```text
src/main.rs           → Entry, route registration, port from config
src/config.rs         → Flat AppConfig (app_key, app_sign, openai_base_url, port)
src/error.rs          → AppError enum, Anthropic-format error responses
src/handler.rs        → Route handlers, AppState, header forwarding
src/anthropic/mod.rs  → Anthropic protocol types
src/openai/mod.rs     → OpenAI protocol types
src/transform/
  ├── request.rs      → Anthropic → OpenAI request conversion
  ├── response.rs     → OpenAI → Anthropic response conversion
  ├── stream.rs       → SSE state machine (chunk → Anthropic events)
  └── headers.rs      → Header forwarding logic
```

## Key behaviors

- Downstream HTTP client timeout: 300s
- `top_k` silently dropped (no OpenAI equivalent)
- Anthropic `system` field → prepended `role: "system"` message
- Stop reason mapping: `stop`→`end_turn`, `length`→`max_tokens`, `tool_calls`→`tool_use`
- All errors use Anthropic error format
- SSE streaming: mpsc channel (buffer 32), split on `\n\n`
- Platform: Windows (`.exe` in gitignore, release builds produce `.exe`)
- Edition: Rust 2024
