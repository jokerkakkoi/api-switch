use crate::config::AppConfig;
use reqwest::Client;
use std::sync::Arc;

pub mod chat_completions;
pub mod health;
pub mod messages;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub client: Client,
}
