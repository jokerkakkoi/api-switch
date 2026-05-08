use crate::anthropic::{AnthropicResponse, AnthropicResponseContentBlock, AnthropicUsage};
use crate::error::AppError;
use crate::openai::{OpenAIErrorResponse, OpenAIResponse};

/// Convert an OpenAI non-streaming response into an Anthropic response.
pub fn convert_response(resp: OpenAIResponse) -> AnthropicResponse {
    let choice = &resp.choices[0];

    let mut content: Vec<AnthropicResponseContentBlock> = Vec::new();

    // Convert text content
    if let Some(ref text) = choice.message.content {
        if !text.is_empty() {
            content.push(AnthropicResponseContentBlock::Text {
                text: text.clone(),
            });
        }
    }

    // Convert tool_calls → tool_use content blocks
    if let Some(ref tool_calls) = choice.message.tool_calls {
        for tc in tool_calls {
            let input: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::json!({}));
            content.push(AnthropicResponseContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                input,
            });
        }
    }

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

    let stop_reason = choice.finish_reason.as_deref().map(map_stop_reason);

    AnthropicResponse {
        id: resp.id,
        response_type: "message".to_string(),
        role: choice.message.role.clone(),
        content,
        model: resp.model,
        stop_reason,
        stop_sequence: None,
        usage,
    }
}

/// Map OpenAI finish_reason to Anthropic stop_reason.
pub fn map_stop_reason(reason: &str) -> String {
    match reason {
        "stop" => "end_turn".to_string(),
        "length" => "max_tokens".to_string(),
        "tool_calls" => "tool_use".to_string(),
        "content_filter" => "content_filter".to_string(),
        _ => reason.to_string(),
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
    use crate::openai::{
        OpenAIChoice, OpenAIFunctionCall, OpenAIResponseMessage, OpenAIToolCallResponse,
        OpenAIUsage,
    };

    fn make_response(content: &str) -> OpenAIResponse {
        OpenAIResponse {
            id: "chatcmpl-123".into(),
            model: "gpt-4".into(),
            choices: vec![OpenAIChoice {
                index: 0,
                message: OpenAIResponseMessage {
                    role: "assistant".into(),
                    content: Some(content.into()),
                    tool_calls: None,
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
        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            AnthropicResponseContentBlock::Text { text } => assert_eq!(text, "Hello!"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_convert_response_usage() {
        let result = convert_response(make_response("Hi"));
        assert_eq!(result.usage.input_tokens, 10);
        assert_eq!(result.usage.output_tokens, 5);
    }

    #[test]
    fn test_convert_response_stop_reason_end_turn() {
        let result = convert_response(make_response("ok"));
        assert_eq!(result.stop_reason, Some("end_turn".into()));
    }

    #[test]
    fn test_convert_response_with_tool_calls() {
        let resp = OpenAIResponse {
            id: "chatcmpl-456".into(),
            model: "gpt-4".into(),
            choices: vec![OpenAIChoice {
                index: 0,
                message: OpenAIResponseMessage {
                    role: "assistant".into(),
                    content: None,
                    tool_calls: Some(vec![OpenAIToolCallResponse {
                        id: "call_abc123".into(),
                        call_type: "function".into(),
                        function: OpenAIFunctionCall {
                            name: "get_weather".into(),
                            arguments: r#"{"location": "NYC"}"#.into(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".into()),
            }],
            usage: Some(OpenAIUsage {
                prompt_tokens: 20,
                completion_tokens: 10,
                total_tokens: 30,
            }),
        };
        let result = convert_response(resp);
        assert_eq!(result.stop_reason, Some("tool_use".into()));
        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            AnthropicResponseContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "call_abc123");
                assert_eq!(name, "get_weather");
                assert_eq!(input["location"], "NYC");
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn test_convert_response_with_text_and_tool_calls() {
        let resp = OpenAIResponse {
            id: "chatcmpl-789".into(),
            model: "gpt-4".into(),
            choices: vec![OpenAIChoice {
                index: 0,
                message: OpenAIResponseMessage {
                    role: "assistant".into(),
                    content: Some("Let me check.".into()),
                    tool_calls: Some(vec![OpenAIToolCallResponse {
                        id: "call_def456".into(),
                        call_type: "function".into(),
                        function: OpenAIFunctionCall {
                            name: "search".into(),
                            arguments: r#"{"query": "weather"}"#.into(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".into()),
            }],
            usage: Some(OpenAIUsage {
                prompt_tokens: 15,
                completion_tokens: 8,
                total_tokens: 23,
            }),
        };
        let result = convert_response(resp);
        // Should have both text and tool_use
        assert_eq!(result.content.len(), 2);
        match &result.content[0] {
            AnthropicResponseContentBlock::Text { text } => assert_eq!(text, "Let me check."),
            _ => panic!("expected Text"),
        }
        match &result.content[1] {
            AnthropicResponseContentBlock::ToolUse { id, name, .. } => {
                assert_eq!(id, "call_def456");
                assert_eq!(name, "search");
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn test_map_stop_reason_tool_calls() {
        assert_eq!(map_stop_reason("tool_calls"), "tool_use");
    }

    #[test]
    fn test_map_stop_reason_all_mappings() {
        assert_eq!(map_stop_reason("stop"), "end_turn");
        assert_eq!(map_stop_reason("length"), "max_tokens");
        assert_eq!(map_stop_reason("tool_calls"), "tool_use");
        assert_eq!(map_stop_reason("content_filter"), "content_filter");
        assert_eq!(map_stop_reason("unknown"), "unknown");
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
