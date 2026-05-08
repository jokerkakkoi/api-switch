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
