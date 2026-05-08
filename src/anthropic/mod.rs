use serde::{Deserialize, Serialize};

/// Incoming Anthropic Messages API request body.
#[derive(Debug, Deserialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub system: Option<AnthropicSystem>,
    pub max_tokens: u32,
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub top_k: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: AnthropicContent,
}

/// Anthropic content can be a plain text string or an array of content blocks.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AnthropicContent {
    TextBlocks(Vec<AnthropicContentBlock>),
    SingleString(String),
}

#[derive(Debug, Deserialize, Clone)]
pub struct AnthropicContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub source: Option<ImageSource>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

/// system can be a plain string or a list of content blocks.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AnthropicSystem {
    String(String),
    ContentList(Vec<AnthropicContentBlock>),
}

/// Outgoing Anthropic response (non-streaming).
#[derive(Debug, Serialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: String,
    pub content: Vec<AnthropicResponseContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: AnthropicUsage,
}

#[derive(Debug, Serialize, Clone)]
pub struct AnthropicResponseContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct AnthropicUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Anthropic SSE event types for streaming.
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type")]
pub enum AnthropicSSEEvent {
    #[serde(rename = "message_start")]
    MessageStart {
        message: AnthropicStreamMessage,
    },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: u32,
        content_block: AnthropicResponseContentBlock,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: u32,
        delta: ContentDelta,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop {
        index: u32,
    },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDeltaData,
        usage: OutputUsage,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
}

#[derive(Debug, Serialize, Clone)]
pub struct AnthropicStreamMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub role: String,
    pub content: Vec<serde_json::Value>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: AnthropicUsage,
}

#[derive(Debug, Serialize, Clone)]
pub struct ContentDelta {
    #[serde(rename = "type")]
    pub delta_type: String,
    pub text: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct MessageDeltaData {
    pub stop_reason: String,
    pub stop_sequence: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct OutputUsage {
    pub output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_anthropic_request_with_string_content() {
        let json = r#"{
            "model": "claude-3-opus",
            "messages": [{"role": "user", "content": "Hello"}],
            "max_tokens": 1024
        }"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "claude-3-opus");
        assert_eq!(req.max_tokens, 1024);
        assert!(!req.stream);
    }

    #[test]
    fn test_deserialize_anthropic_request_with_stream() {
        let json = r#"{
            "model": "claude-3",
            "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}],
            "max_tokens": 500,
            "stream": true
        }"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        assert!(req.stream);
    }

    #[test]
    fn test_deserialize_system_as_string() {
        let json = r#"{
            "model": "claude-3",
            "messages": [{"role": "user", "content": "hi"}],
            "system": "You are helpful.",
            "max_tokens": 100
        }"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        match req.system.unwrap() {
            AnthropicSystem::String(s) => assert_eq!(s, "You are helpful."),
            _ => panic!("expected string system"),
        }
    }
}
