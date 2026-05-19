use serde::Deserialize;
use std::env;

use crate::error::AppError;

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

    pub fn require_model(&self, name: &str) -> Result<&ModelConfig, AppError> {
        self.find_model(name)
            .ok_or_else(|| AppError::InvalidRequestError(format!("Unknown model: {}", name)))
    }
}

pub fn load_config() -> AppConfig {
    let path = env::var("CONFIG_PATH").unwrap_or_else(|_| "./config.yaml".to_string());
    load_config_from_path(&path)
}

pub fn load_config_from_path(path: &str) -> AppConfig {
    let contents = std::fs::read_to_string(path)
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
    fn load_config_reads_from_custom_path() {
        let yaml = r#"
models:
  - name: test-model
    app_key: "custom-key"
    app_sign: "custom-sign"
    base_url: http://custom/
port: 5000
"#;
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_config_custom.yaml");
        std::fs::write(&file_path, yaml).unwrap();

        let config = load_config_from_path(file_path.to_str().unwrap());

        assert_eq!(config.models.len(), 1);
        assert_eq!(config.models[0].name, "test-model");
        assert_eq!(config.models[0].app_key, "custom-key");
        assert_eq!(config.port, 5000);

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    fn load_config_uses_default_port_when_omitted() {
        let yaml = r#"
models:
  - name: no-port-model
    app_key: "key"
    app_sign: "sign"
    base_url: http://host/
"#;
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_config_no_port.yaml");
        std::fs::write(&file_path, yaml).unwrap();

        let config = load_config_from_path(file_path.to_str().unwrap());

        assert_eq!(config.port, 3000);

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    #[should_panic(expected = "Failed to read config file")]
    fn load_config_panics_on_missing_file() {
        let _ = load_config_from_path("/nonexistent/path/config.yaml");
    }

    #[test]
    #[should_panic(expected = "Failed to parse config file")]
    fn load_config_panics_on_invalid_yaml() {
        let invalid_yaml = "this is: not: valid: yaml: [";
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_config_invalid.yaml");
        std::fs::write(&file_path, invalid_yaml).unwrap();

        let _ = load_config_from_path(file_path.to_str().unwrap());

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    #[should_panic(expected = "Failed to parse config file")]
    fn load_config_panics_on_missing_required_fields() {
        let yaml = r#"
models:
  - name: incomplete
"#;
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_config_incomplete.yaml");
        std::fs::write(&file_path, yaml).unwrap();

        let _ = load_config_from_path(file_path.to_str().unwrap());

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    fn load_config_handles_empty_models_list() {
        let yaml = r#"
models: []
port: 8080
"#;
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_config_empty_models.yaml");
        std::fs::write(&file_path, yaml).unwrap();

        let config = load_config_from_path(file_path.to_str().unwrap());

        assert!(config.models.is_empty());
        assert_eq!(config.port, 8080);

        std::fs::remove_file(&file_path).ok();
    }
}
