use crate::anthropic::{
    AnthropicContent, AnthropicContentBlock, AnthropicRequest, AnthropicSystem, AnthropicTool,
    AnthropicToolChoice, ToolResultContent,
};
use crate::openai::{
    ImageUrl, OpenAIContent, OpenAIContentBlock, OpenAIFunction, OpenAIFunctionCall, OpenAIMessage,
    OpenAIRequest, OpenAITool, OpenAIToolCallRequest,
};

pub fn convert_request(req: AnthropicRequest) -> OpenAIRequest {
    let mut messages = Vec::new();

    // Anthropic system field → OpenAI system-role message
    if let Some(system) = req.system {
        let system_text = match system {
            AnthropicSystem::String(s) => s,
            AnthropicSystem::ContentList(blocks) => blocks
                .into_iter()
                .filter_map(|b| b.text)
                .collect::<Vec<_>>()
                .join("\n\n"),
        };
        messages.push(OpenAIMessage {
            role: "system".to_string(),
            content: OpenAIContent::Text(system_text),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    // Convert messages
    for msg in req.messages {
        convert_message(msg.role, msg.content, &mut messages);
    }

    // Convert tools
    let tools = req.tools.map(|t| t.into_iter().map(convert_tool).collect());

    // Convert tool_choice
    let tool_choice = req.tool_choice.map(convert_tool_choice);

    OpenAIRequest {
        model: req.model,
        messages,
        max_tokens: Some(req.max_tokens),
        stop: req.stop_sequences,
        temperature: req.temperature,
        top_p: req.top_p,
        stream: req.stream,
        tools,
        tool_choice,
    }
}

/// Convert a single Anthropic message into one or more OpenAI messages.
/// tool_result blocks in user messages are extracted as separate role:"tool" messages.
/// tool_use blocks in assistant messages are converted to tool_calls field.
fn convert_message(role: String, content: AnthropicContent, messages: &mut Vec<OpenAIMessage>) {
    match content {
        AnthropicContent::SingleString(text) => {
            messages.push(OpenAIMessage {
                role,
                content: OpenAIContent::Text(text),
                tool_calls: None,
                tool_call_id: None,
            });
        }
        AnthropicContent::TextBlocks(blocks) => {
            if role == "assistant" {
                convert_assistant_blocks(blocks, messages);
            } else if role == "user" {
                convert_user_blocks(blocks, messages);
            } else {
                // For other roles, just convert content normally
                let content = convert_content_blocks(blocks);
                messages.push(OpenAIMessage {
                    role,
                    content,
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }
    }
}

/// Convert assistant message content blocks.
/// Separates text/image blocks from tool_use blocks.
fn convert_assistant_blocks(blocks: Vec<AnthropicContentBlock>, messages: &mut Vec<OpenAIMessage>) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<OpenAIToolCallRequest> = Vec::new();

    for block in blocks {
        match block {
            AnthropicContentBlock::Text { text } => {
                text_parts.push(text);
            }
            AnthropicContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(OpenAIToolCallRequest {
                    id,
                    call_type: "function".to_string(),
                    function: OpenAIFunctionCall {
                        name,
                        arguments: serde_json::to_string(&input).unwrap_or_default(),
                    },
                });
            }
            _ => {}
        }
    }

    let content = if text_parts.is_empty() {
        OpenAIContent::Null
    } else {
        OpenAIContent::Text(text_parts.join(""))
    };

    let tool_calls_opt = if tool_calls.is_empty() {
        None
    } else {
        Some(tool_calls)
    };

    messages.push(OpenAIMessage {
        role: "assistant".to_string(),
        content,
        tool_calls: tool_calls_opt,
        tool_call_id: None,
    });
}

/// Convert user message content blocks.
/// tool_result blocks are extracted as separate role:"tool" messages.
/// Other blocks become a normal user message.
fn convert_user_blocks(blocks: Vec<AnthropicContentBlock>, messages: &mut Vec<OpenAIMessage>) {
    let mut normal_blocks: Vec<AnthropicContentBlock> = Vec::new();
    let mut tool_results: Vec<(String, String)> = Vec::new(); // (tool_use_id, content)

    for block in blocks {
        match block {
            AnthropicContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                let result_text = match content {
                    Some(ToolResultContent::Text(t)) => t,
                    Some(ToolResultContent::Blocks(blocks)) => blocks
                        .into_iter()
                        .filter_map(|b| b.text)
                        .collect::<Vec<_>>()
                        .join("\n"),
                    None => String::new(),
                };
                tool_results.push((tool_use_id, result_text));
            }
            other => {
                normal_blocks.push(other);
            }
        }
    }

    // Emit tool result messages first (role: "tool")
    for (tool_use_id, content) in tool_results {
        messages.push(OpenAIMessage {
            role: "tool".to_string(),
            content: OpenAIContent::Text(content),
            tool_calls: None,
            tool_call_id: Some(tool_use_id),
        });
    }

    // Emit remaining user content (if any)
    if !normal_blocks.is_empty() {
        let content = convert_content_blocks(normal_blocks);
        messages.push(OpenAIMessage {
            role: "user".to_string(),
            content,
            tool_calls: None,
            tool_call_id: None,
        });
    }
}

/// Convert a list of (non-tool) content blocks to OpenAI content.
fn convert_content_blocks(blocks: Vec<AnthropicContentBlock>) -> OpenAIContent {
    // If single text block, use simple Text variant
    if blocks.len() == 1 {
        if let AnthropicContentBlock::Text { ref text } = blocks[0] {
            return OpenAIContent::Text(text.clone());
        }
    }
    // Multi-block → convert each
    let converted: Vec<OpenAIContentBlock> =
        blocks.into_iter().map(convert_content_block).collect();
    OpenAIContent::MultiContent(converted)
}

fn convert_content_block(block: AnthropicContentBlock) -> OpenAIContentBlock {
    match block {
        AnthropicContentBlock::Image { source } => {
            let url = format!("data:{};base64,{}", source.media_type, source.data);
            OpenAIContentBlock {
                content_type: "image_url".to_string(),
                text: None,
                image_url: Some(ImageUrl { url }),
            }
        }
        AnthropicContentBlock::Text { text } => OpenAIContentBlock {
            content_type: "text".to_string(),
            text: Some(text),
            image_url: None,
        },
        // tool_use and tool_result should have been handled earlier
        _ => OpenAIContentBlock {
            content_type: "text".to_string(),
            text: Some(String::new()),
            image_url: None,
        },
    }
}

/// Convert Anthropic tool definition to OpenAI format.
fn convert_tool(tool: AnthropicTool) -> OpenAITool {
    OpenAITool {
        tool_type: "function".to_string(),
        function: OpenAIFunction {
            name: tool.name,
            description: tool.description,
            parameters: tool.input_schema,
        },
    }
}

/// Convert Anthropic tool_choice to OpenAI format.
fn convert_tool_choice(choice: AnthropicToolChoice) -> serde_json::Value {
    match choice {
        AnthropicToolChoice::Auto => serde_json::json!("auto"),
        AnthropicToolChoice::Any => serde_json::json!("required"),
        AnthropicToolChoice::Tool { name } => serde_json::json!({
            "type": "function",
            "function": { "name": name }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::{AnthropicMessage, AnthropicSystem};

    fn make_request(stream: bool) -> AnthropicRequest {
        AnthropicRequest {
            model: "claude-3-opus".into(),
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: AnthropicContent::SingleString("Hello!".into()),
            }],
            system: Some(AnthropicSystem::String("You are helpful.".into())),
            max_tokens: 1024,
            stop_sequences: None,
            stream,
            temperature: Some(0.7),
            top_p: None,
            top_k: Some(5),
            tools: None,
            tool_choice: None,
        }
    }

    #[test]
    fn test_convert_basic_request() {
        let result = convert_request(make_request(false));
        assert_eq!(result.model, "claude-3-opus");
        assert_eq!(result.messages.len(), 2); // system + user
        assert_eq!(result.messages[0].role, "system");
        match &result.messages[0].content {
            OpenAIContent::Text(t) => assert_eq!(t, "You are helpful."),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_convert_stop_sequences() {
        let mut req = make_request(false);
        req.stop_sequences = Some(vec!["END".into(), "STOP".into()]);
        let result = convert_request(req);
        assert_eq!(result.stop, Some(vec!["END".into(), "STOP".into()]));
    }

    #[test]
    fn test_convert_stream_flag() {
        let result = convert_request(make_request(true));
        assert!(result.stream);
    }

    #[test]
    fn test_top_k_is_dropped() {
        let mut req = make_request(false);
        req.top_k = Some(10);
        let result = convert_request(req);
        // top_k has no OpenAI equivalent — just verify conversion succeeds
        assert_eq!(result.model, "claude-3-opus");
    }

    // ===== Tools conversion tests =====

    #[test]
    fn test_convert_tools_definition() {
        let mut req = make_request(false);
        req.tools = Some(vec![AnthropicTool {
            name: "get_weather".into(),
            description: Some("Get current weather".into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string", "description": "City name"}
                },
                "required": ["location"]
            }),
        }]);
        let result = convert_request(req);
        let tools = result.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_type, "function");
        assert_eq!(tools[0].function.name, "get_weather");
        assert_eq!(
            tools[0].function.description.as_ref().unwrap(),
            "Get current weather"
        );
        assert_eq!(tools[0].function.parameters["type"], "object");
    }

    #[test]
    fn test_convert_tool_choice_auto() {
        let mut req = make_request(false);
        req.tool_choice = Some(AnthropicToolChoice::Auto);
        let result = convert_request(req);
        assert_eq!(result.tool_choice.unwrap(), serde_json::json!("auto"));
    }

    #[test]
    fn test_convert_tool_choice_any() {
        let mut req = make_request(false);
        req.tool_choice = Some(AnthropicToolChoice::Any);
        let result = convert_request(req);
        assert_eq!(result.tool_choice.unwrap(), serde_json::json!("required"));
    }

    #[test]
    fn test_convert_tool_choice_specific_tool() {
        let mut req = make_request(false);
        req.tool_choice = Some(AnthropicToolChoice::Tool {
            name: "get_weather".into(),
        });
        let result = convert_request(req);
        let expected = serde_json::json!({
            "type": "function",
            "function": { "name": "get_weather" }
        });
        assert_eq!(result.tool_choice.unwrap(), expected);
    }

    #[test]
    fn test_convert_tool_use_in_assistant_message() {
        let req = AnthropicRequest {
            model: "claude-3".into(),
            messages: vec![AnthropicMessage {
                role: "assistant".into(),
                content: AnthropicContent::TextBlocks(vec![
                    AnthropicContentBlock::Text {
                        text: "I'll check the weather.".into(),
                    },
                    AnthropicContentBlock::ToolUse {
                        id: "toolu_01".into(),
                        name: "get_weather".into(),
                        input: serde_json::json!({"location": "NYC"}),
                    },
                ]),
            }],
            system: None,
            max_tokens: 1024,
            stop_sequences: None,
            stream: false,
            temperature: None,
            top_p: None,
            top_k: None,
            tools: None,
            tool_choice: None,
        };
        let result = convert_request(req);
        assert_eq!(result.messages.len(), 1);
        let msg = &result.messages[0];
        assert_eq!(msg.role, "assistant");

        // Check text content
        match &msg.content {
            OpenAIContent::Text(t) => assert_eq!(t, "I'll check the weather."),
            _ => panic!("expected Text content"),
        }

        // Check tool_calls
        let tool_calls = msg.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "toolu_01");
        assert_eq!(tool_calls[0].call_type, "function");
        assert_eq!(tool_calls[0].function.name, "get_weather");
        let args: serde_json::Value =
            serde_json::from_str(&tool_calls[0].function.arguments).unwrap();
        assert_eq!(args["location"], "NYC");
    }

    #[test]
    fn test_convert_tool_result_in_user_message() {
        let req = AnthropicRequest {
            model: "claude-3".into(),
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: AnthropicContent::TextBlocks(vec![AnthropicContentBlock::ToolResult {
                    tool_use_id: "toolu_01".into(),
                    content: Some(ToolResultContent::Text("72°F and sunny".into())),
                    is_error: None,
                }]),
            }],
            system: None,
            max_tokens: 1024,
            stop_sequences: None,
            stream: false,
            temperature: None,
            top_p: None,
            top_k: None,
            tools: None,
            tool_choice: None,
        };
        let result = convert_request(req);
        // tool_result becomes a separate "tool" role message
        assert_eq!(result.messages.len(), 1);
        let msg = &result.messages[0];
        assert_eq!(msg.role, "tool");
        assert_eq!(msg.tool_call_id.as_ref().unwrap(), "toolu_01");
        match &msg.content {
            OpenAIContent::Text(t) => assert_eq!(t, "72°F and sunny"),
            _ => panic!("expected Text content"),
        }
    }

    #[test]
    fn test_convert_multiple_tool_results_with_text() {
        let req = AnthropicRequest {
            model: "claude-3".into(),
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: AnthropicContent::TextBlocks(vec![
                    AnthropicContentBlock::ToolResult {
                        tool_use_id: "toolu_01".into(),
                        content: Some(ToolResultContent::Text("Result 1".into())),
                        is_error: None,
                    },
                    AnthropicContentBlock::ToolResult {
                        tool_use_id: "toolu_02".into(),
                        content: Some(ToolResultContent::Text("Result 2".into())),
                        is_error: None,
                    },
                    AnthropicContentBlock::Text {
                        text: "Now continue.".into(),
                    },
                ]),
            }],
            system: None,
            max_tokens: 1024,
            stop_sequences: None,
            stream: false,
            temperature: None,
            top_p: None,
            top_k: None,
            tools: None,
            tool_choice: None,
        };
        let result = convert_request(req);
        // 2 tool messages + 1 user message
        assert_eq!(result.messages.len(), 3);
        assert_eq!(result.messages[0].role, "tool");
        assert_eq!(
            result.messages[0].tool_call_id.as_ref().unwrap(),
            "toolu_01"
        );
        assert_eq!(result.messages[1].role, "tool");
        assert_eq!(
            result.messages[1].tool_call_id.as_ref().unwrap(),
            "toolu_02"
        );
        assert_eq!(result.messages[2].role, "user");
        match &result.messages[2].content {
            OpenAIContent::Text(t) => assert_eq!(t, "Now continue."),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_convert_image_content_block() {
        let req = AnthropicRequest {
            model: "claude-3".into(),
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: AnthropicContent::TextBlocks(vec![
                    AnthropicContentBlock::Text {
                        text: "What is in this image?".into(),
                    },
                    AnthropicContentBlock::Image {
                        source: crate::anthropic::ImageSource {
                            source_type: "base64".into(),
                            media_type: "image/png".into(),
                            data: "iVBOR...".into(),
                        },
                    },
                ]),
            }],
            system: None,
            max_tokens: 1024,
            stop_sequences: None,
            stream: false,
            temperature: None,
            top_p: None,
            top_k: None,
            tools: None,
            tool_choice: None,
        };
        let result = convert_request(req);
        assert_eq!(result.messages.len(), 1);
        match &result.messages[0].content {
            OpenAIContent::MultiContent(blocks) => {
                assert_eq!(blocks.len(), 2);
                assert_eq!(blocks[0].content_type, "text");
                assert_eq!(blocks[1].content_type, "image_url");
                let url = &blocks[1].image_url.as_ref().unwrap().url;
                assert!(url.starts_with("data:image/png;base64,"));
            }
            _ => panic!("expected MultiContent"),
        }
    }

    #[test]
    fn test_convert_no_system() {
        let req = AnthropicRequest {
            model: "claude-3".into(),
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: AnthropicContent::SingleString("hi".into()),
            }],
            system: None,
            max_tokens: 100,
            stop_sequences: None,
            stream: false,
            temperature: None,
            top_p: None,
            top_k: None,
            tools: None,
            tool_choice: None,
        };
        let result = convert_request(req);
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].role, "user");
    }

    #[test]
    fn test_convert_system_content_list() {
        let req = AnthropicRequest {
            model: "claude-3".into(),
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: AnthropicContent::SingleString("hi".into()),
            }],
            system: Some(AnthropicSystem::ContentList(vec![
                crate::anthropic::SystemContentBlock {
                    content_type: "text".into(),
                    text: Some("Part one.".into()),
                },
                crate::anthropic::SystemContentBlock {
                    content_type: "text".into(),
                    text: Some("Part two.".into()),
                },
            ])),
            max_tokens: 100,
            stop_sequences: None,
            stream: false,
            temperature: None,
            top_p: None,
            top_k: None,
            tools: None,
            tool_choice: None,
        };
        let result = convert_request(req);
        assert_eq!(result.messages.len(), 2);
        match &result.messages[0].content {
            OpenAIContent::Text(t) => assert_eq!(t, "Part one.\n\nPart two."),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn test_convert_assistant_tool_use_only_no_text() {
        let req = AnthropicRequest {
            model: "claude-3".into(),
            messages: vec![AnthropicMessage {
                role: "assistant".into(),
                content: AnthropicContent::TextBlocks(vec![AnthropicContentBlock::ToolUse {
                    id: "toolu_01".into(),
                    name: "search".into(),
                    input: serde_json::json!({"query": "rust"}),
                }]),
            }],
            system: None,
            max_tokens: 1024,
            stop_sequences: None,
            stream: false,
            temperature: None,
            top_p: None,
            top_k: None,
            tools: None,
            tool_choice: None,
        };
        let result = convert_request(req);
        let msg = &result.messages[0];
        // Content should be Null when no text
        match &msg.content {
            OpenAIContent::Null => {}
            _ => panic!("expected Null content for tool_calls-only message"),
        }
        assert!(msg.tool_calls.is_some());
    }
}
