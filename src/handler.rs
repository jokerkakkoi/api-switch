use std::sync::Arc;

use crate::config::{AppConfig, ModelConfig};
use crate::error::AppError;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub client: reqwest::Client,
}

pub fn resolve_model<'a>(state: &'a AppState, model_name: &str) -> Result<&'a ModelConfig, AppError> {
    state
        .config
        .find_model(model_name)
        .ok_or_else(|| AppError::InvalidRequestError(format!("Unknown model: {}", model_name)))
}
