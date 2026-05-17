# Design: Multi-Model Routing

## Summary

Route requests to different backend models based on the model field in the request body. Each model has its own credentials (app_key, app_sign) and base_url. Old flat config fields are removed entirely.

## Config Format

```yaml
models:
  - name: qwen35-397b
    app_key: "19031anl30JsPAO71xI1"
    app_sign: "f58ff37650ea7288766b5e32f6ec9c1756617358"
    base_url: http://10.142.78.100:9520/ai_platform/inferservice/member1/qwen35-397b/
  - name: glm-5
    app_key: "19031a7oOXDOQaP5ry79"
    app_sign: "9858d4a7d32e441219ea0d3cf23e1009a46b46f4"
    base_url: http://10.142.78.100:9520/ai_platform_b/inferservice/member1/glm5
port: 3000
```

## Architecture

### config.rs

- **ModelConfig** struct: name: String, app_key: String, app_sign: String, base_url: String
- **AppConfig** struct: models: Vec<ModelConfig>, port: u16
- **AppConfig::find_model(&self, name: &str) -> Option<&ModelConfig>** - linear search by name
- Remove old base_url() free function
- Remove old flat fields: app_key, app_sign, openai_base_url

### handler.rs

- **AppState**: remove base_url field. Keep config: Arc<AppConfig>, client: Client
- **messages_handler**:
  1. Parse body to extract model field (deserialize full AnthropicRequest)
  2. state.config.find_model(&model) - lookup
  3. If None: return 400 InvalidRequestError("Unknown model: {model}")
  4. If Some(mc): use mc.app_key, mc.app_sign, mc.base_url for forwarding
- **chat_completions_handler**:
  1. Parse body as serde_json::Value to extract model field
  2. Same lookup logic as above
  3. Use matched model's config for forwarding
- **health_handler**: use the first configured model's base_url for the health check backend URL

### main.rs

- Remove config::base_url() call
- AppState no longer holds base_url

### Error Behavior

| Scenario | Response |
|----------|----------|
| Model not found in config | 400 - invalid_request_error with "Unknown model: <name>" |
| Backend error | Existing AppError mapping (unchanged) |

## Testing Strategy (TDD)

### config.rs tests
- find_model_returns_matching_config - exact name match
- find_model_returns_none_for_unknown - no match
- find_model_is_case_sensitive - "Qwen35" != "qwen35-397b"
- config_loads_from_yaml - integration test with multi-model YAML

### handler.rs tests
- messages_handler_unknown_model_returns_400 - unknown model name
- chat_completions_handler_unknown_model_returns_400 - unknown model name
- messages_handler_known_model_forwards_with_correct_credentials - verify app-key/app-sign from matched model
- chat_completions_handler_known_model_forwards_to_correct_base_url - verify URL from matched model

## File Changes

| File | Change |
|------|--------|
| src/config.rs | New ModelConfig, new AppConfig, find_model(), remove base_url() |
| src/handler.rs | Model lookup in both handlers, remove base_url from AppState |
| src/main.rs | Remove base_url from AppState construction |
| config.yaml.example | Update to multi-model format |

## Risks

- **Linear lookup performance**: Acceptable for small model lists (< 100). If it grows, can switch to HashMap later.
- **Body parsing in chat_completions_handler**: Currently passes raw body string. Need to parse JSON to extract model field, then forward the original body unchanged.
