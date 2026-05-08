use serde::Deserialize;
use std::env;

fn default_port() -> u16 {
    3000
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub app_key: String,
    pub app_sign: String,
    pub openai_base_url: Option<String>,
    #[serde(default = "default_port")]
    pub port: u16,
}

/// Load config from CONFIG_PATH env var, fallback to ./config.yaml
pub fn load_config() -> AppConfig {
    let path = env::var("CONFIG_PATH").unwrap_or_else(|_| "./config.yaml".to_string());
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read config file '{}': {}", path, e));
    serde_yaml::from_str(&contents)
        .unwrap_or_else(|e| panic!("Failed to parse config file '{}': {}", path, e))
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
    fn test_base_url_from_config() {
        let config = AppConfig {
            app_key: "key-123".to_string(),
            app_sign: "sign-abc".to_string(),
            openai_base_url: Some("http://custom-url.example.com".to_string()),
            port: 3000,
        };
        assert_eq!(base_url(&config), "http://custom-url.example.com");
    }

    #[test]
    fn test_base_url_default() {
        let config = AppConfig {
            app_key: "key-123".to_string(),
            app_sign: "sign-abc".to_string(),
            openai_base_url: None,
            port: 3000,
        };
        assert_eq!(base_url(&config), "https://api.openai.com");
    }
}
