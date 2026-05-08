use serde::{Deserialize, Serialize};

/// Request body sent to OpenAI-compatible backend.
#[derive(Debug, Serialize)]
pub struct OpenAIRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OpenAITool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: OpenAIContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAIToolCallRequest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// String for simple text, Vec for multi-content (text+image), Null for tool_calls-only messages.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum OpenAIContent {
    Text(String),
    MultiContent(Vec<OpenAIContentBlock>),
    Null,
}

#[derive(Debug, Serialize)]
pub struct OpenAIContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<ImageUrl>,
}

#[derive(Debug, Serialize)]
pub struct ImageUrl {
    pub url: String,
}

/// Tool definition in OpenAI format.
#[derive(Debug, Serialize, Clone)]
pub struct OpenAITool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OpenAIFunction,
}

#[derive(Debug, Serialize, Clone)]
pub struct OpenAIFunction {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: serde_json::Value,
}

/// Tool call in request messages (for assistant messages with tool_calls).
#[derive(Debug, Serialize, Clone)]
pub struct OpenAIToolCallRequest {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAIFunctionCall,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OpenAIFunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Non-streaming response from OpenAI backend.
#[derive(Debug, Deserialize)]
pub struct OpenAIResponse {
    pub id: String,
    pub model: String,
    pub choices: Vec<OpenAIChoice>,
    pub usage: Option<OpenAIUsage>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIChoice {
    pub index: u32,
    pub message: OpenAIResponseMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIResponseMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<OpenAIToolCallResponse>>,
}

/// Tool call in response messages.
#[derive(Debug, Deserialize, Clone)]
pub struct OpenAIToolCallResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAIFunctionCall,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Single SSE chunk from OpenAI streaming response.
#[derive(Debug, Deserialize)]
pub struct OpenAISSEChunk {
    pub id: Option<String>,
    pub model: Option<String>,
    pub choices: Option<Vec<OpenAIDeltaChoice>>,
    pub usage: Option<OpenAIUsage>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIDeltaChoice {
    pub index: u32,
    pub delta: OpenAIDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIDelta {
    pub role: Option<String>,
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<OpenAIDeltaToolCall>>,
}

/// Incremental tool call data in SSE delta.
#[derive(Debug, Deserialize, Clone)]
pub struct OpenAIDeltaToolCall {
    pub index: u32,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub call_type: Option<String>,
    #[serde(default)]
    pub function: Option<OpenAIDeltaFunction>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OpenAIDeltaFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

/// Error response from OpenAI backend.
#[derive(Debug, Deserialize)]
pub struct OpenAIErrorResponse {
    pub error: OpenAIErrorDetail,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_openai_request() {
        let req = OpenAIRequest {
            model: "gpt-4".into(),
            messages: vec![OpenAIMessage {
                role: "user".into(),
                content: OpenAIContent::Text("hi".into()),
                tool_calls: None,
                tool_call_id: None,
            }],
            max_tokens: Some(100),
            stop: None,
            temperature: None,
            top_p: None,
            stream: false,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"model\":\"gpt-4\""));
        assert!(json.contains("\"stream\":false"));
        // tools and tool_choice should not appear when None
        assert!(!json.contains("\"tools\""));
        assert!(!json.contains("\"tool_choice\""));
    }

    #[test]
    fn test_serialize_openai_request_with_tools() {
        let req = OpenAIRequest {
            model: "gpt-4".into(),
            messages: vec![OpenAIMessage {
                role: "user".into(),
                content: OpenAIContent::Text("weather?".into()),
                tool_calls: None,
                tool_call_id: None,
            }],
            max_tokens: Some(100),
            stop: None,
            temperature: None,
            top_p: None,
            stream: false,
            tools: Some(vec![OpenAITool {
                tool_type: "function".into(),
                function: OpenAIFunction {
                    name: "get_weather".into(),
                    description: Some("Get weather".into()),
                    parameters: serde_json::json!({"type": "object", "properties": {}}),
                },
            }]),
            tool_choice: Some(serde_json::json!("auto")),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"tools\""));
        assert!(json.contains("\"get_weather\""));
        assert!(json.contains("\"tool_choice\":\"auto\""));
    }

    #[test]
    fn test_deserialize_openai_response() {
        let json = r#"{
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello!"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        }"#;
        let resp: OpenAIResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "chatcmpl-123");
        assert_eq!(resp.choices[0].message.content.as_ref().unwrap(), "Hello!");
    }

    #[test]
    fn test_deserialize_openai_response_with_tool_calls() {
        let json = r#"{
            "id": "chatcmpl-456",
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\": \"NYC\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 10, "total_tokens": 30}
        }"#;
        let resp: OpenAIResponse = serde_json::from_str(json).unwrap();
        let tool_calls = resp.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_abc123");
        assert_eq!(tool_calls[0].function.name, "get_weather");
        assert_eq!(resp.choices[0].finish_reason.as_ref().unwrap(), "tool_calls");
    }

    #[test]
    fn test_deserialize_sse_chunk_with_delta() {
        let json = r#"{
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{"index": 0, "delta": {"content": "Hi"}, "finish_reason": null}]
        }"#;
        let chunk: OpenAISSEChunk = serde_json::from_str(json).unwrap();
        let choices = chunk.choices.unwrap();
        assert_eq!(choices[0].delta.content.as_ref().unwrap(), "Hi");
    }

    #[test]
    fn test_deserialize_sse_chunk_with_tool_calls() {
        let json = r#"{
            "id": "chatcmpl-789",
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_xyz",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": ""}
                    }]
                },
                "finish_reason": null
            }]
        }"#;
        let chunk: OpenAISSEChunk = serde_json::from_str(json).unwrap();
        let choices = chunk.choices.unwrap();
        let tool_calls = choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls[0].id.as_ref().unwrap(), "call_xyz");
        assert_eq!(
            tool_calls[0].function.as_ref().unwrap().name.as_ref().unwrap(),
            "get_weather"
        );
    }
}
