use serde::Deserialize;
use std::collections::HashMap;
use std::env;

fn default_port() -> u16 {
    3000
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub tokens: HashMap<String, TokenCredentials>,
    pub openai_base_url: Option<String>,
    #[serde(default = "default_port")]
    pub port: u16,
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

/// Get base URL from config.yaml openai_base_url, fallback to OpenAI default.
pub fn base_url(config: &AppConfig) -> String {
    config
        .openai_base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com".to_string())
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
        let config = AppConfig {
            tokens,
            openai_base_url: None,
            port: 3000,
        };
        let result = lookup_token(&config, "sk-ant-xxx");
        assert!(result.is_some());
        assert_eq!(result.unwrap().app_key, "key-123");
    }

    #[test]
    fn test_lookup_token_not_found() {
        let config = AppConfig {
            tokens: HashMap::new(),
            openai_base_url: None,
            port: 3000,
        };
        assert!(lookup_token(&config, "unknown").is_none());
    }

    #[test]
    fn test_base_url_from_config() {
        let config = AppConfig {
            tokens: HashMap::new(),
            openai_base_url: Some("http://custom-url.example.com".to_string()),
            port: 3000,
        };
        assert_eq!(base_url(&config), "http://custom-url.example.com");
    }

    #[test]
    fn test_base_url_default() {
        let config = AppConfig {
            tokens: HashMap::new(),
            openai_base_url: None,
            port: 3000,
        };
        assert_eq!(base_url(&config), "https://api.openai.com");
    }
}
