use crate::anthropic::{AnthropicResponse, AnthropicResponseContentBlock, AnthropicUsage};
use crate::error::AppError;
use crate::openai::{OpenAIErrorResponse, OpenAIResponse};

/// Convert an OpenAI non-streaming response into an Anthropic response.
pub fn convert_response(resp: OpenAIResponse) -> AnthropicResponse {
    let choice = &resp.choices[0];
    let content = match &choice.message.content {
        Some(text) => vec![AnthropicResponseContentBlock {
            content_type: "text".to_string(),
            text: text.clone(),
        }],
        None => vec![],
    };

    let usage = match &resp.usage {
        Some(u) => AnthropicUsage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
        },
        None => AnthropicUsage {
            input_tokens: 0,
            output_tokens: 0,
        },
    };

    AnthropicResponse {
        id: resp.id,
        response_type: "message".to_string(),
        role: choice.message.role.clone(),
        content,
        model: resp.model,
        stop_reason: choice.finish_reason.clone(),
        stop_sequence: None,
        usage,
    }
}

/// Parse an OpenAI error body and map it to our AppError type.
pub fn map_openai_error(status: u16, body: &str) -> AppError {
    if let Ok(err_resp) = serde_json::from_str::<OpenAIErrorResponse>(body) {
        let msg = err_resp.error.message;
        match status {
            401 | 403 => AppError::AuthenticationError(msg),
            429 => AppError::RateLimitError(msg),
            _ => AppError::ApiError(msg),
        }
    } else {
        match status {
            401 | 403 => AppError::AuthenticationError(format!("Backend returned {}", status)),
            429 => AppError::RateLimitError(format!("Backend returned {}", status)),
            _ => AppError::ApiError(format!("Backend returned {}: {}", status, body)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{OpenAIChoice, OpenAIResponseMessage, OpenAIUsage};

    fn make_response(content: &str) -> OpenAIResponse {
        OpenAIResponse {
            id: "chatcmpl-123".into(),
            model: "gpt-4".into(),
            choices: vec![OpenAIChoice {
                index: 0,
                message: OpenAIResponseMessage {
                    role: "assistant".into(),
                    content: Some(content.into()),
                },
                finish_reason: Some("stop".into()),
            }],
            usage: Some(OpenAIUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            }),
        }
    }

    #[test]
    fn test_convert_response_content() {
        let result = convert_response(make_response("Hello!"));
        assert_eq!(result.id, "chatcmpl-123");
        assert_eq!(result.role, "assistant");
        assert_eq!(result.content[0].content_type, "text");
        assert_eq!(result.content[0].text, "Hello!");
    }

    #[test]
    fn test_convert_response_usage() {
        let result = convert_response(make_response("Hi"));
        assert_eq!(result.usage.input_tokens, 10);
        assert_eq!(result.usage.output_tokens, 5);
    }

    #[test]
    fn test_convert_response_stop_reason() {
        let result = convert_response(make_response("ok"));
        assert_eq!(result.stop_reason, Some("stop".into()));
    }

    #[test]
    fn test_map_openai_error_auth() {
        let body = r#"{"error":{"message":"Bad API key","type":"invalid_request_error"}}"#;
        let err = map_openai_error(401, body);
        match err {
            AppError::AuthenticationError(msg) => assert!(msg.contains("Bad API key")),
            _ => panic!("expected AuthenticationError"),
        }
    }

    #[test]
    fn test_map_openai_error_rate_limit() {
        let body = r#"{"error":{"message":"Rate limited","type":"rate_limit"}}"#;
        let err = map_openai_error(429, body);
        match err {
            AppError::RateLimitError(msg) => assert!(msg.contains("Rate limited")),
            _ => panic!("expected RateLimitError"),
        }
    }

    #[test]
    fn test_map_openai_error_server_error() {
        let err = map_openai_error(500, "Internal error");
        match err {
            AppError::ApiError(_) => {}
            _ => panic!("expected ApiError"),
        }
    }
}
