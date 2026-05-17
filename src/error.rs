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

    #[test]
    fn test_invalid_request_error_returns_400() {
        let response = AppError::InvalidRequestError("bad input".into()).into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_api_error_returns_502() {
        let response = AppError::ApiError("backend failed".into()).into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn test_timeout_error_returns_504() {
        let response = AppError::TimeoutError("slow backend".into()).into_response();
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    }

    #[tokio::test]
    async fn test_error_response_body_structure() {
        let response = AppError::AuthenticationError("bad token".into()).into_response();
        let (_, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["error"]["type"], "authentication_error");
        assert_eq!(json["error"]["message"], "bad token");
    }

    #[tokio::test]
    async fn test_from_reqwest_timeout_error() {
        let client = reqwest::Client::new();
        let result = client
            .get("http://127.0.0.1:1")
            .timeout(std::time::Duration::from_millis(1))
            .send()
            .await;
        let e = result.expect_err("expected request to fail");
        let app_err = AppError::from(e);
        match app_err {
            AppError::TimeoutError(_) | AppError::ApiError(_) => {}
            _ => panic!("expected TimeoutError or ApiError"),
        }
    }

    #[tokio::test]
    async fn test_from_reqwest_connect_error() {
        let client = reqwest::Client::new();
        let result = client
            .get("http://127.0.0.1:1")
            .timeout(std::time::Duration::from_millis(50))
            .send()
            .await;
        let e = result.expect_err("expected request to fail");
        let app_err = AppError::from(e);
        match app_err {
            AppError::TimeoutError(_) | AppError::ApiError(_) => {}
            _ => panic!("expected TimeoutError or ApiError"),
        }
    }

    #[test]
    fn test_from_axum_http_error() {
        let err_result = axum::http::Response::builder()
            .header("invalid\x01header", "value")
            .body(());
        let e = err_result.expect_err("expected request to fail");
        let app_err = AppError::from(e);
        match app_err {
            AppError::ApiError(msg) => assert!(msg.contains("Failed to build response")),
            _ => panic!("expected ApiError"),
        }
    }
}
