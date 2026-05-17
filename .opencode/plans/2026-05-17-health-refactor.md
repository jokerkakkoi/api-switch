# Health Endpoint Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor `/health` to return gateway self-health status and per-model network connectivity checks.

**Architecture:** Replace the current proxy-based health handler with a new handler that (1) always returns `health: true` for the gateway itself, and (2) uses `tokio::task::JoinSet` to spawn concurrent tasks for each configured model's `base_url` HTTP GET check with a 1s timeout, returning a JSON response with per-model boolean status.

**Tech Stack:** Rust, axum, reqwest, serde, serde_json, tokio

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `src/handler.rs` | Modify | Replace `health_handler` and `resolve_health_base_url` with new health check logic |
| `src/handler.rs` (tests) | Modify | Replace existing health test with new tests for the refactored endpoint |

No new files needed. The existing `handler.rs` contains the health handler and its tests. The `AppState` already has `config` and `client` which are sufficient.

---

### Task 1: Implement new health response struct and handler

**Files:**
- Modify: `src/handler.rs:1-43` (add new struct), `src/handler.rs:302-323` (replace handler)

- [ ] **Step 1: Add health response struct**

Add a new serializable struct for the health response near the top of `handler.rs`, after the existing imports and `AppState` definition (around line 28):

```rust
use serde::Serialize;

#[derive(Serialize)]
struct HealthResponse {
    health: bool,
    network: std::collections::HashMap<String, bool>,
}
```

- [ ] **Step 2: Replace `resolve_health_base_url` function**

Delete the existing `resolve_health_base_url` function (lines 37-43). It will no longer be needed.

- [ ] **Step 3: Replace `health_handler` function**

Replace the existing `health_handler` (lines 302-323) with the new implementation:

```rust
/// GET /health — gateway self-health and per-model network checks
pub async fn health_handler(State(state): State<AppState>) -> Result<Json<HealthResponse>, AppError> {
    let health_timeout = std::time::Duration::from_secs(1);

    let mut join_set = tokio::task::JoinSet::new();

    for model in &state.config.models {
        let name = model.name.clone();
        let base_url = model.base_url.trim_end_matches('/').to_string();
        let client = state.client.clone();
        join_set.spawn(async move {
            let reachable = client
                .get(&base_url)
                .timeout(health_timeout)
                .send()
                .await
                .is_ok();
            (name, reachable)
        });
    }

    let mut network = std::collections::HashMap::new();
    while let Some(result) = join_set.join_next().await {
        if let Ok((name, reachable)) = result {
            network.insert(name, reachable);
        }
    }

    Ok(Json(HealthResponse {
        health: true,
        network,
    }))
}
```

- [ ] **Step 4: Run tests to verify compilation**

Run: `cargo build`
Expected: Compiles successfully (existing tests will fail, but code should compile)

- [ ] **Step 5: Commit**

```bash
git add src/handler.rs
git commit -m "refactor: replace health handler with gateway self-health and per-model network checks"
```

---

### Task 2: Update health handler tests

**Files:**
- Modify: `src/handler.rs` (tests section, replace `health_handler_returns_backend_status` test and add new tests)

- [ ] **Step 1: Replace existing health test**

Find and replace the existing `health_handler_returns_backend_status` test (lines 1031-1077) with these new tests:

```rust
    #[tokio::test]
    async fn health_handler_returns_gateway_health_and_network_status() {
        let app = Router::new().route(
            "/",
            get(|| async move { axum::http::StatusCode::OK }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let backend_url = format!("http://{}", addr);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let config = Arc::new(AppConfig {
            models: vec![
                ModelConfig {
                    name: "qwen35-397b".into(),
                    app_key: "key".into(),
                    app_sign: "sign".into(),
                    base_url: backend_url.clone(),
                },
                ModelConfig {
                    name: "glm-5".into(),
                    app_key: "key2".into(),
                    app_sign: "sign2".into(),
                    base_url: backend_url,
                },
            ],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/health", get(health_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .get(format!("http://{}/health", proxy_addr))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 200);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["health"], true);
        assert_eq!(body["network"]["qwen35-397b"], true);
        assert_eq!(body["network"]["glm-5"], true);
    }

    #[tokio::test]
    async fn health_handler_network_false_for_unreachable_model() {
        let config = Arc::new(AppConfig {
            models: vec![
                ModelConfig {
                    name: "qwen35-397b".into(),
                    app_key: "key".into(),
                    app_sign: "sign".into(),
                    base_url: "http://127.0.0.1:1".into(), // unreachable port
                },
            ],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/health", get(health_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .get(format!("http://{}/health", proxy_addr))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 200);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["health"], true);
        assert_eq!(body["network"]["qwen35-397b"], false);
    }

    #[tokio::test]
    async fn health_handler_empty_models_returns_empty_network() {
        let config = Arc::new(AppConfig {
            models: vec![],
            port: 0,
        });
        let state = AppState {
            config,
            client: Client::new(),
        };
        let app = Router::new()
            .route("/health", get(health_handler))
            .with_state(state);

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(proxy_listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let res = client
            .get(format!("http://{}/health", proxy_addr))
            .send()
            .await
            .unwrap();

        assert_eq!(res.status(), 200);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body["health"], true);
        assert!(body["network"].as_object().unwrap().is_empty());
    }
```

- [ ] **Step 2: Run the new health tests**

Run: `cargo test -- handler::tests::health_handler --nocapture`
Expected: All 3 tests pass

- [ ] **Step 3: Run all tests to verify nothing is broken**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/handler.rs
git commit -m "test: update health handler tests for new response format"
```

---

## Self-Review

**1. Spec coverage:**
- "只做本网关服务健康检查，返回true" → `health: true` always returned ✓
- "对配置文件中每个模型的openai endpoint做网络检查" → Iterates all `config.models`, sends HTTP GET to each `base_url` ✓
- "如果有http返回则返回true。否则返回false" → `.is_ok()` on `send().await` determines boolean ✓
- Reference response format matches: `{"health": true, "network": {"qwen35-397b": true, "glm-5": true}}` ✓

**2. Placeholder scan:** No TBD, TODO, or placeholder patterns found. All code is complete.

**3. Type consistency:** 
- `HealthResponse` uses `Serialize` derive (serde already imported)
- Uses `std::collections::HashMap` (standard library, no new dependency)
- Uses `tokio::task::JoinSet` for concurrent execution (tokio already has "full" features)
- `AppState` already has `config: Arc<AppConfig>` and `client: Client` — no changes needed
- Handler returns `Result<Json<HealthResponse>, AppError>` — consistent with other handlers
- Test code uses same patterns as existing tests (`TcpListener`, `axum::serve`, `reqwest::Client`)
