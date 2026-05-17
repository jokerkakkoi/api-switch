use serde::Deserialize;
use std::env;

fn default_port() -> u16 {
    3000
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelConfig {
    pub name: String,
    pub app_key: String,
    pub app_sign: String,
    pub base_url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub models: Vec<ModelConfig>,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl AppConfig {
    pub fn find_model(&self, name: &str) -> Option<&ModelConfig> {
        self.models.iter().find(|m| m.name == name)
    }
}

pub fn load_config() -> AppConfig {
    let path = env::var("CONFIG_PATH").unwrap_or_else(|_| "./config.yaml".to_string());
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read config file '{}': {}", path, e));
    serde_yaml::from_str(&contents)
        .unwrap_or_else(|e| panic!("Failed to parse config file '{}': {}", path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_model_returns_matching_config() {
        let config = AppConfig {
            models: vec![
                ModelConfig {
                    name: "qwen35-397b".into(),
                    app_key: "key1".into(),
                    app_sign: "sign1".into(),
                    base_url: "http://host1".into(),
                },
                ModelConfig {
                    name: "glm-5".into(),
                    app_key: "key2".into(),
                    app_sign: "sign2".into(),
                    base_url: "http://host2".into(),
                },
            ],
            port: 3000,
        };
        let result = config.find_model("glm-5");
        assert!(result.is_some());
        let mc = result.unwrap();
        assert_eq!(mc.app_key, "key2");
        assert_eq!(mc.app_sign, "sign2");
        assert_eq!(mc.base_url, "http://host2");
    }

    #[test]
    fn find_model_returns_none_for_unknown() {
        let config = AppConfig {
            models: vec![ModelConfig {
                name: "qwen35-397b".into(),
                app_key: "key1".into(),
                app_sign: "sign1".into(),
                base_url: "http://host1".into(),
            }],
            port: 3000,
        };
        assert!(config.find_model("unknown-model").is_none());
    }

    #[test]
    fn find_model_is_case_sensitive() {
        let config = AppConfig {
            models: vec![ModelConfig {
                name: "qwen35-397b".into(),
                app_key: "key1".into(),
                app_sign: "sign1".into(),
                base_url: "http://host1".into(),
            }],
            port: 3000,
        };
        assert!(config.find_model("Qwen35-397b").is_none());
        assert!(config.find_model("qwen35-397b").is_some());
    }

    #[test]
    fn config_loads_from_yaml() {
        let yaml = r#"
models:
  - name: qwen35-397b
    app_key: "key1"
    app_sign: "sign1"
    base_url: http://host1/
  - name: glm-5
    app_key: "key2"
    app_sign: "sign2"
    base_url: http://host2/
port: 4000
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.models.len(), 2);
        assert_eq!(config.models[0].name, "qwen35-397b");
        assert_eq!(config.models[1].name, "glm-5");
        assert_eq!(config.port, 4000);
        assert!(config.find_model("qwen35-397b").is_some());
        let qwen35_config = config.find_model("qwen35-397b").unwrap();
        assert_eq!(qwen35_config.app_key, "key1");
        assert_eq!(qwen35_config.app_sign, "sign1");
        assert_eq!(qwen35_config.base_url, "http://host1/");
        assert!(config.find_model("glm-5").is_some());
        assert!(config.find_model("missing").is_none());
    }
}
