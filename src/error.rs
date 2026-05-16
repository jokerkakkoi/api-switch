use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug)]
pub enum AppError {
    AuthenticationError(String),
    InvalidRequestError(String),
    RateLimitError(String),
    ApiError(String),
    TimeoutError(String),
}

#[derive(Serialize)]
struct AnthropicErrorResponse {
    #[serde(rename = "type")]
    error_type: String,
    error: AnthropicErrorDetail,
}

#[derive(Serialize)]
struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match &self {
            AppError::AuthenticationError(msg) => (
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                msg.clone(),
            ),
            AppError::InvalidRequestError(msg) => (
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                msg.clone(),
            ),
            AppError::RateLimitError(msg) => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limit_error",
                msg.clone(),
            ),
            AppError::ApiError(msg) => (StatusCode::BAD_GATEWAY, "api_error", msg.clone()),
            AppError::TimeoutError(msg) => {
                (StatusCode::GATEWAY_TIMEOUT, "timeout_error", msg.clone())
            }
        };

        let body = AnthropicErrorResponse {
            error_type: "error".to_string(),
            error: AnthropicErrorDetail {
                error_type: error_type.to_string(),
                message,
            },
        };

        (status, Json(body)).into_response()
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            AppError::TimeoutError("Request to backend timed out".to_string())
        } else if err.is_connect() {
            AppError::ApiError("Failed to connect to backend".to_string())
        } else {
            AppError::ApiError(format!("Backend request failed: {}", err))
        }
    }
}

impl From<axum::http::Error> for AppError {
    fn from(err: axum::http::Error) -> Self {
        AppError::ApiError(format!("Failed to build response: {}", err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_error_returns_401() {
        let response = AppError::AuthenticationError("bad token".into()).into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_rate_limit_error_returns_429() {
        let response = AppError::RateLimitError("too many".into()).into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
