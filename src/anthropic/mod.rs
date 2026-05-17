use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    #[serde(default)]
    pub tools: Option<Vec<AnthropicTool>>,
    #[serde(default)]
    pub tool_choice: Option<AnthropicToolChoice>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
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
#[serde(tag = "type")]
pub enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: Option<ToolResultContent>,
        #[serde(default)]
        is_error: Option<bool>,
    },
}

/// tool_result content can be a string or an array of content blocks.
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(String),
    Blocks(Vec<ToolResultContentBlock>),
}

#[derive(Debug, Deserialize, Clone)]
pub struct ToolResultContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
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
    ContentList(Vec<SystemContentBlock>),
}

/// System content block (simplified, only text type).
#[derive(Debug, Deserialize)]
pub struct SystemContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
}

/// Tool definition in Anthropic format.
/// Also accepts OpenAI-style tools ({ "type": "function", "function": { ... } })
/// by deserializing from either format.
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum AnthropicTool {
    /// OpenAI-style format: { "type": "function", "function": { "name": ..., "parameters": ... } }
    OpenAIStyle {
        #[serde(rename = "type")]
        _tool_type: String,
        function: AnthropicToolFunction,
    },
    /// Anthropic native format: { "name": ..., "input_schema": ... }
    Native {
        name: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(rename = "input_schema")]
        input_schema: serde_json::Value,
    },
}

#[derive(Debug, Deserialize, Clone)]
pub struct AnthropicToolFunction {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
}

impl AnthropicTool {
    pub fn name(&self) -> &str {
        match self {
            AnthropicTool::OpenAIStyle { function, .. } => &function.name,
            AnthropicTool::Native { name, .. } => name,
        }
    }

    pub fn description(&self) -> Option<&str> {
        match self {
            AnthropicTool::OpenAIStyle { function, .. } => function.description.as_deref(),
            AnthropicTool::Native { description, .. } => description.as_deref(),
        }
    }

    pub fn input_schema(&self) -> serde_json::Value {
        match self {
            AnthropicTool::OpenAIStyle { function, .. } => {
                function.parameters.clone().unwrap_or(serde_json::json!({}))
            }
            AnthropicTool::Native { input_schema, .. } => input_schema.clone(),
        }
    }
}

/// Tool choice in Anthropic format.
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum AnthropicToolChoice {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "tool")]
    Tool { name: String },
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
#[serde(tag = "type")]
pub enum AnthropicResponseContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
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
    MessageStart { message: AnthropicStreamMessage },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: u32,
        content_block: AnthropicResponseContentBlock,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: u32, delta: ContentDelta },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },
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
#[serde(tag = "type")]
pub enum ContentDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
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

    #[test]
    fn test_deserialize_request_with_tools() {
        let json = r#"{
            "model": "claude-3",
            "messages": [{"role": "user", "content": "What is the weather?"}],
            "max_tokens": 1024,
            "tools": [
                {
                    "name": "get_weather",
                    "description": "Get the current weather",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "location": {"type": "string"}
                        },
                        "required": ["location"]
                    }
                }
            ],
            "tool_choice": {"type": "auto"}
        }"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        let tools = req.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "get_weather");
        match req.tool_choice.unwrap() {
            AnthropicToolChoice::Auto => {}
            _ => panic!("expected Auto"),
        }
    }

    #[test]
    fn test_deserialize_request_with_openai_style_tools() {
        let json = r#"{
            "model": "claude-3",
            "messages": [{"role": "user", "content": "What is the weather?"}],
            "max_tokens": 1024,
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get the current weather",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "location": {"type": "string"}
                            },
                            "required": ["location"]
                        }
                    }
                }
            ],
            "tool_choice": {"type": "auto"}
        }"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        let tools = req.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "get_weather");
        assert_eq!(tools[0].description().unwrap(), "Get the current weather");
        assert_eq!(tools[0].input_schema()["type"], "object");
        match req.tool_choice.unwrap() {
            AnthropicToolChoice::Auto => {}
            _ => panic!("expected Auto"),
        }
    }

    #[test]
    fn test_deserialize_request_with_extra_fields() {
        let json = r#"{
            "model": "claude-3",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1024,
            "metadata": {"user_id": "test"},
            "thinking": {"type": "adaptive"},
            "context_management": {"edits": []}
        }"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.extra["metadata"]["user_id"], "test");
        assert_eq!(req.extra["thinking"]["type"], "adaptive");
        assert!(req.extra.contains_key("context_management"));
    }

    #[test]
    fn test_deserialize_tool_use_content_block() {
        let json = r#"[
            {"type": "text", "text": "I'll check the weather."},
            {"type": "tool_use", "id": "toolu_01", "name": "get_weather", "input": {"location": "NYC"}}
        ]"#;
        let blocks: Vec<AnthropicContentBlock> = serde_json::from_str(json).unwrap();
        assert_eq!(blocks.len(), 2);
        match &blocks[1] {
            AnthropicContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_01");
                assert_eq!(name, "get_weather");
                assert_eq!(input["location"], "NYC");
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn test_deserialize_tool_result_content_block() {
        let json = r#"[
            {"type": "tool_result", "tool_use_id": "toolu_01", "content": "72°F and sunny"}
        ]"#;
        let blocks: Vec<AnthropicContentBlock> = serde_json::from_str(json).unwrap();
        match &blocks[0] {
            AnthropicContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                assert_eq!(tool_use_id, "toolu_01");
                match content.as_ref().unwrap() {
                    ToolResultContent::Text(t) => assert_eq!(t, "72°F and sunny"),
                    _ => panic!("expected text content"),
                }
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn test_deserialize_tool_choice_variants() {
        let auto: AnthropicToolChoice = serde_json::from_str(r#"{"type": "auto"}"#).unwrap();
        assert!(matches!(auto, AnthropicToolChoice::Auto));

        let any: AnthropicToolChoice = serde_json::from_str(r#"{"type": "any"}"#).unwrap();
        assert!(matches!(any, AnthropicToolChoice::Any));

        let tool: AnthropicToolChoice =
            serde_json::from_str(r#"{"type": "tool", "name": "get_weather"}"#).unwrap();
        match tool {
            AnthropicToolChoice::Tool { name } => assert_eq!(name, "get_weather"),
            _ => panic!("expected Tool"),
        }
    }

    #[test]
    fn test_serialize_response_with_tool_use() {
        let resp = AnthropicResponse {
            id: "msg_123".into(),
            response_type: "message".into(),
            role: "assistant".into(),
            content: vec![
                AnthropicResponseContentBlock::Text {
                    text: "Let me check.".into(),
                },
                AnthropicResponseContentBlock::ToolUse {
                    id: "toolu_01".into(),
                    name: "get_weather".into(),
                    input: serde_json::json!({"location": "NYC"}),
                },
            ],
            model: "claude-3".into(),
            stop_reason: Some("tool_use".into()),
            stop_sequence: None,
            usage: AnthropicUsage {
                input_tokens: 100,
                output_tokens: 50,
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"tool_use\""));
        assert!(json.contains("\"get_weather\""));
    }

    #[test]
    fn test_deserialize_system_as_content_list() {
        let json = r#"{
            "model": "claude-3",
            "messages": [{"role": "user", "content": "hi"}],
            "system": [{"type": "text", "text": "You are helpful."}],
            "max_tokens": 100
        }"#;
        let req: AnthropicRequest = serde_json::from_str(json).unwrap();
        match req.system.unwrap() {
            AnthropicSystem::ContentList(blocks) => {
                assert_eq!(blocks.len(), 1);
                assert_eq!(blocks[0].text.as_ref().unwrap(), "You are helpful.");
            }
            _ => panic!("expected ContentList"),
        }
    }

    #[test]
    fn test_deserialize_image_content_block() {
        let json = r#"[
            {"type": "text", "text": "What is this?"},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "iVBOR..."}}
        ]"#;
        let blocks: Vec<AnthropicContentBlock> = serde_json::from_str(json).unwrap();
        assert_eq!(blocks.len(), 2);
        match &blocks[1] {
            AnthropicContentBlock::Image { source } => {
                assert_eq!(source.media_type, "image/png");
                assert_eq!(source.data, "iVBOR...");
            }
            _ => panic!("expected Image"),
        }
    }

    #[test]
    fn test_deserialize_tool_result_with_is_error_and_blocks() {
        let json = r#"[
            {"type": "tool_result", "tool_use_id": "toolu_01", "is_error": true, "content": [{"type": "text", "text": "Error occurred"}]}
        ]"#;
        let blocks: Vec<AnthropicContentBlock> = serde_json::from_str(json).unwrap();
        match &blocks[0] {
            AnthropicContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "toolu_01");
                assert_eq!(is_error.unwrap(), true);
                match content.as_ref().unwrap() {
                    ToolResultContent::Blocks(blks) => {
                        assert_eq!(blks.len(), 1);
                        assert_eq!(blks[0].text.as_ref().unwrap(), "Error occurred");
                    }
                    _ => panic!("expected Blocks"),
                }
            }
            _ => panic!("expected ToolResult"),
        }
    }
}
