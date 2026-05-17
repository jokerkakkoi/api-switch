use crate::anthropic::{
    AnthropicResponseContentBlock, AnthropicSSEEvent, AnthropicStreamMessage, AnthropicUsage,
    ContentDelta, MessageDeltaData, OutputUsage,
};
use crate::openai::OpenAISSEChunk;
use crate::transform::response::map_stop_reason;
use uuid::Uuid;

/// Tracks state for an in-progress tool call in the stream.
#[derive(Debug, Clone)]
struct ToolCallState {
    id: String,
    name: String,
    arguments_buffer: String,
}

/// Tracks state across SSE chunks to produce the correct Anthropic event sequence.
pub struct StreamState {
    pub message_id: String,
    pub model: String,
    pub input_tokens: u32,
    pub content_index: u32,
    pub started: bool,
    pub content_block_open: bool,
    /// Tracks in-progress tool calls by their OpenAI index.
    tool_calls: std::collections::HashMap<u32, ToolCallState>,
    /// Whether the current open block is a tool_use block.
    tool_block_open: bool,
}

impl StreamState {
    pub fn new() -> Self {
        Self {
            message_id: format!("msg_{}", &Uuid::new_v4().to_string().replace('-', "")[..24]),
            model: String::new(),
            input_tokens: 0,
            content_index: 0,
            started: false,
            content_block_open: false,
            tool_calls: std::collections::HashMap::new(),
            tool_block_open: false,
        }
    }
}

/// Convert a single OpenAI SSE chunk into zero or more Anthropic SSE events.
pub fn convert_stream_chunk(
    chunk: &OpenAISSEChunk,
    state: &mut StreamState,
) -> Vec<AnthropicSSEEvent> {
    let mut events = Vec::new();

    // Capture model from first chunk
    if let Some(ref model) = chunk.model {
        if state.model.is_empty() {
            state.model = model.clone();
        }
    }

    // Capture prompt token count
    if let Some(ref usage) = chunk.usage {
        state.input_tokens = usage.prompt_tokens;
    }

    // On first chunk with data, emit message_start
    if !state.started {
        events.push(AnthropicSSEEvent::MessageStart {
            message: AnthropicStreamMessage {
                id: state.message_id.clone(),
                msg_type: "message".to_string(),
                role: "assistant".to_string(),
                content: vec![],
                model: state.model.clone(),
                stop_reason: None,
                stop_sequence: None,
                usage: AnthropicUsage {
                    input_tokens: state.input_tokens,
                    output_tokens: 0,
                },
            },
        });
        state.started = true;
    }

    // Process choice deltas
    if let Some(ref choices) = chunk.choices {
        for choice in choices {
            // Role delta → start content block (text)
            if choice.delta.role.is_some() && !state.content_block_open {
                events.push(AnthropicSSEEvent::ContentBlockStart {
                    index: state.content_index,
                    content_block: AnthropicResponseContentBlock::Text {
                        text: String::new(),
                    },
                });
                state.content_block_open = true;
                state.tool_block_open = false;
            }

            // Text content delta
            if let Some(ref text) = choice.delta.content {
                if !state.content_block_open {
                    events.push(AnthropicSSEEvent::ContentBlockStart {
                        index: state.content_index,
                        content_block: AnthropicResponseContentBlock::Text {
                            text: String::new(),
                        },
                    });
                    state.content_block_open = true;
                    state.tool_block_open = false;
                }
                events.push(AnthropicSSEEvent::ContentBlockDelta {
                    index: state.content_index,
                    delta: ContentDelta::TextDelta { text: text.clone() },
                });
            }

            // Tool calls delta
            if let Some(ref tool_calls) = choice.delta.tool_calls {
                for tc in tool_calls {
                    let tc_index = tc.index;

                    // Check if this is a new tool call (has id) or continuation
                    if let Some(ref id) = tc.id {
                        // Close previous content block if open
                        if state.content_block_open {
                            events.push(AnthropicSSEEvent::ContentBlockStop {
                                index: state.content_index,
                            });
                            state.content_index += 1;
                            state.content_block_open = false;
                        }

                        // Get function name
                        let name = tc
                            .function
                            .as_ref()
                            .and_then(|f| f.name.clone())
                            .unwrap_or_default();

                        // Store tool call state
                        state.tool_calls.insert(
                            tc_index,
                            ToolCallState {
                                id: id.clone(),
                                name: name.clone(),
                                arguments_buffer: String::new(),
                            },
                        );

                        // Emit content_block_start for tool_use
                        events.push(AnthropicSSEEvent::ContentBlockStart {
                            index: state.content_index,
                            content_block: AnthropicResponseContentBlock::ToolUse {
                                id: id.clone(),
                                name,
                                input: serde_json::json!({}),
                            },
                        });
                        state.content_block_open = true;
                        state.tool_block_open = true;
                    }

                    // Arguments delta
                    if let Some(ref func) = tc.function {
                        if let Some(ref args) = func.arguments {
                            if !args.is_empty() {
                                // Accumulate arguments
                                if let Some(tc_state) = state.tool_calls.get_mut(&tc_index) {
                                    tc_state.arguments_buffer.push_str(args);
                                }

                                events.push(AnthropicSSEEvent::ContentBlockDelta {
                                    index: state.content_index,
                                    delta: ContentDelta::InputJsonDelta {
                                        partial_json: args.clone(),
                                    },
                                });
                            }
                        }
                    }
                }
            }

            // Finish reason → end content block + message stop
            if let Some(ref finish_reason) = choice.finish_reason {
                if state.content_block_open {
                    events.push(AnthropicSSEEvent::ContentBlockStop {
                        index: state.content_index,
                    });
                    state.content_block_open = false;
                }

                let stop_reason = map_stop_reason(finish_reason);
                let output_tokens = chunk
                    .usage
                    .as_ref()
                    .map(|u| u.completion_tokens)
                    .unwrap_or(0);

                events.push(AnthropicSSEEvent::MessageDelta {
                    delta: MessageDeltaData {
                        stop_reason,
                        stop_sequence: None,
                    },
                    usage: OutputUsage { output_tokens },
                });

                events.push(AnthropicSSEEvent::MessageStop);
            }
        }
    }

    events
}

/// Format an Anthropic SSE event into SSE wire format.
pub fn format_sse(event: &AnthropicSSEEvent) -> String {
    let event_type = match event {
        AnthropicSSEEvent::MessageStart { .. } => "message_start",
        AnthropicSSEEvent::ContentBlockStart { .. } => "content_block_start",
        AnthropicSSEEvent::ContentBlockDelta { .. } => "content_block_delta",
        AnthropicSSEEvent::ContentBlockStop { .. } => "content_block_stop",
        AnthropicSSEEvent::MessageDelta { .. } => "message_delta",
        AnthropicSSEEvent::MessageStop => "message_stop",
        AnthropicSSEEvent::Ping => "ping",
    };
    let data = serde_json::to_string(event).unwrap();
    format!("event: {}\ndata: {}\n\n", event_type, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{
        OpenAIDelta, OpenAIDeltaChoice, OpenAIDeltaFunction, OpenAIDeltaToolCall, OpenAIUsage,
    };

    #[test]
    fn test_stream_state_new() {
        let state = StreamState::new();
        assert!(!state.started);
        assert!(!state.content_block_open);
        assert_eq!(state.content_index, 0);
    }

    #[test]
    fn test_first_chunk_with_role_emits_start_and_block_start() {
        let chunk = make_chunk(Some("assistant"), None, None, None, None, None);
        let mut state = StreamState::new();
        let events = convert_stream_chunk(&chunk, &mut state);

        assert!(state.started);
        assert!(state.content_block_open);
        let has_msg_start = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::MessageStart { .. }));
        let has_block_start = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockStart { .. }));
        assert!(has_msg_start);
        assert!(has_block_start);
    }

    #[test]
    fn test_content_delta_emits_delta_event() {
        let chunk = make_chunk(None, Some("Hello"), None, None, None, None);
        let mut state = StreamState::new();
        let events = convert_stream_chunk(&chunk, &mut state);

        let has_delta = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockDelta { .. }));
        assert!(has_delta);
    }

    #[test]
    fn test_finish_reason_emits_stop_events() {
        let usage = OpenAIUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        };
        let chunk = make_chunk(None, None, Some("stop"), Some("gpt-4"), Some(usage), None);
        let mut state = StreamState::new();
        state.started = true;
        let events = convert_stream_chunk(&chunk, &mut state);

        let has_msg_delta = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::MessageDelta { .. }));
        let has_msg_stop = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::MessageStop));
        assert!(has_msg_delta);
        assert!(has_msg_stop);
    }

    #[test]
    fn test_format_sse_ping() {
        let event = AnthropicSSEEvent::Ping;
        let output = format_sse(&event);
        assert!(output.starts_with("event: ping\n"));
        assert!(output.ends_with("\n\n"));
    }

    #[test]
    fn test_map_stop_reason_mappings() {
        assert_eq!(map_stop_reason("stop"), "end_turn");
        assert_eq!(map_stop_reason("length"), "max_tokens");
        assert_eq!(map_stop_reason("tool_calls"), "tool_use");
    }

    // ===== Tool call streaming tests =====

    #[test]
    fn test_stream_tool_call_start() {
        let chunk = make_chunk(
            None,
            None,
            None,
            Some("gpt-4"),
            None,
            Some(vec![OpenAIDeltaToolCall {
                index: 0,
                id: Some("call_abc".into()),
                call_type: Some("function".into()),
                function: Some(OpenAIDeltaFunction {
                    name: Some("get_weather".into()),
                    arguments: Some(String::new()),
                }),
            }]),
        );
        let mut state = StreamState::new();
        state.started = true;
        let events = convert_stream_chunk(&chunk, &mut state);

        // Should emit content_block_start with tool_use type
        let has_tool_start = events.iter().any(|e| match e {
            AnthropicSSEEvent::ContentBlockStart { content_block, .. } => {
                matches!(content_block, AnthropicResponseContentBlock::ToolUse { name, .. } if name == "get_weather")
            }
            _ => false,
        });
        assert!(has_tool_start);
        assert!(state.content_block_open);
        assert!(state.tool_block_open);
    }

    #[test]
    fn test_stream_tool_call_arguments_delta() {
        let mut state = StreamState::new();
        state.started = true;

        // First chunk: tool call start
        let chunk1 = make_chunk(
            None,
            None,
            None,
            None,
            None,
            Some(vec![OpenAIDeltaToolCall {
                index: 0,
                id: Some("call_abc".into()),
                call_type: Some("function".into()),
                function: Some(OpenAIDeltaFunction {
                    name: Some("get_weather".into()),
                    arguments: Some(String::new()),
                }),
            }]),
        );
        convert_stream_chunk(&chunk1, &mut state);

        // Second chunk: arguments delta
        let chunk2 = make_chunk(
            None,
            None,
            None,
            None,
            None,
            Some(vec![OpenAIDeltaToolCall {
                index: 0,
                id: None,
                call_type: None,
                function: Some(OpenAIDeltaFunction {
                    name: None,
                    arguments: Some(r#"{"loc"#.into()),
                }),
            }]),
        );
        let events = convert_stream_chunk(&chunk2, &mut state);

        let has_json_delta = events.iter().any(|e| match e {
            AnthropicSSEEvent::ContentBlockDelta { delta, .. } => {
                matches!(delta, ContentDelta::InputJsonDelta { partial_json } if partial_json == r#"{"loc"#)
            }
            _ => false,
        });
        assert!(has_json_delta);
    }

    #[test]
    fn test_stream_tool_call_finish() {
        let mut state = StreamState::new();
        state.started = true;

        // Start tool call
        let chunk1 = make_chunk(
            None,
            None,
            None,
            None,
            None,
            Some(vec![OpenAIDeltaToolCall {
                index: 0,
                id: Some("call_abc".into()),
                call_type: Some("function".into()),
                function: Some(OpenAIDeltaFunction {
                    name: Some("get_weather".into()),
                    arguments: Some(r#"{"location":"NYC"}"#.into()),
                }),
            }]),
        );
        convert_stream_chunk(&chunk1, &mut state);

        // Finish reason
        let chunk2 = make_chunk(None, None, Some("tool_calls"), None, None, None);
        let events = convert_stream_chunk(&chunk2, &mut state);

        // Should have content_block_stop + message_delta with tool_use + message_stop
        let has_block_stop = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockStop { .. }));
        let has_msg_delta = events.iter().any(|e| match e {
            AnthropicSSEEvent::MessageDelta { delta, .. } => delta.stop_reason == "tool_use",
            _ => false,
        });
        let has_msg_stop = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::MessageStop));

        assert!(has_block_stop);
        assert!(has_msg_delta);
        assert!(has_msg_stop);
    }

    #[test]
    fn test_stream_multiple_tool_calls() {
        let mut state = StreamState::new();
        state.started = true;

        // First tool call
        let chunk1 = make_chunk(
            None,
            None,
            None,
            None,
            None,
            Some(vec![OpenAIDeltaToolCall {
                index: 0,
                id: Some("call_1".into()),
                call_type: Some("function".into()),
                function: Some(OpenAIDeltaFunction {
                    name: Some("tool_a".into()),
                    arguments: Some(r#"{}"#.into()),
                }),
            }]),
        );
        let events1 = convert_stream_chunk(&chunk1, &mut state);
        let start_count_1 = events1
            .iter()
            .filter(|e| matches!(e, AnthropicSSEEvent::ContentBlockStart { .. }))
            .count();
        assert_eq!(start_count_1, 1);

        // Second tool call (new id, should close previous block and start new one)
        let chunk2 = make_chunk(
            None,
            None,
            None,
            None,
            None,
            Some(vec![OpenAIDeltaToolCall {
                index: 1,
                id: Some("call_2".into()),
                call_type: Some("function".into()),
                function: Some(OpenAIDeltaFunction {
                    name: Some("tool_b".into()),
                    arguments: Some(r#"{}"#.into()),
                }),
            }]),
        );
        let events2 = convert_stream_chunk(&chunk2, &mut state);

        // Should have content_block_stop (for tool_a) + content_block_start (for tool_b)
        let has_stop = events2
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockStop { .. }));
        let has_start = events2
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockStart { .. }));
        assert!(has_stop);
        assert!(has_start);
        assert_eq!(state.content_index, 1); // Should have incremented
    }

    #[test]
    fn test_stream_full_text_sequence() {
        let mut state = StreamState::new();

        // 1. Role chunk
        let c1 = make_chunk(Some("assistant"), None, None, Some("gpt-4"), None, None);
        let e1 = convert_stream_chunk(&c1, &mut state);
        assert!(
            e1.iter()
                .any(|e| matches!(e, AnthropicSSEEvent::MessageStart { .. }))
        );
        assert!(
            e1.iter()
                .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockStart { .. }))
        );

        // 2. Content deltas
        let c2 = make_chunk(None, Some("Hello"), None, None, None, None);
        let e2 = convert_stream_chunk(&c2, &mut state);
        assert!(
            e2.iter()
                .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockDelta { .. }))
        );

        let c3 = make_chunk(None, Some(" world"), None, None, None, None);
        let e3 = convert_stream_chunk(&c3, &mut state);
        assert!(
            e3.iter()
                .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockDelta { .. }))
        );

        // 3. Finish
        let c4 = make_chunk(None, None, Some("stop"), None, None, None);
        let e4 = convert_stream_chunk(&c4, &mut state);
        assert!(
            e4.iter()
                .any(|e| matches!(e, AnthropicSSEEvent::ContentBlockStop { .. }))
        );
        assert!(
            e4.iter()
                .any(|e| matches!(e, AnthropicSSEEvent::MessageDelta { .. }))
        );
        assert!(
            e4.iter()
                .any(|e| matches!(e, AnthropicSSEEvent::MessageStop))
        );
    }

    #[test]
    fn test_convert_stream_chunk_no_choices() {
        let chunk = OpenAISSEChunk {
            id: Some("chatcmpl-123".into()),
            model: Some("gpt-4".into()),
            choices: None,
            usage: None,
        };
        let mut state = StreamState::new();
        let events = convert_stream_chunk(&chunk, &mut state);

        // Should still emit message_start on first chunk
        assert!(state.started);
        assert_eq!(state.model, "gpt-4");
        let has_msg_start = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::MessageStart { .. }));
        assert!(has_msg_start);
        // No content block start since there's no delta
        assert!(!state.content_block_open);
    }

    #[test]
    fn test_stream_tool_call_arguments_accumulation() {
        let mut state = StreamState::new();
        state.started = true;

        // Start tool call
        let chunk1 = make_chunk(
            None,
            None,
            None,
            None,
            None,
            Some(vec![OpenAIDeltaToolCall {
                index: 0,
                id: Some("call_abc".into()),
                call_type: Some("function".into()),
                function: Some(OpenAIDeltaFunction {
                    name: Some("get_weather".into()),
                    arguments: Some(String::new()),
                }),
            }]),
        );
        convert_stream_chunk(&chunk1, &mut state);

        // First arguments delta
        let chunk2 = make_chunk(
            None,
            None,
            None,
            None,
            None,
            Some(vec![OpenAIDeltaToolCall {
                index: 0,
                id: None,
                call_type: None,
                function: Some(OpenAIDeltaFunction {
                    name: None,
                    arguments: Some("{\"location\"".into()),
                }),
            }]),
        );
        convert_stream_chunk(&chunk2, &mut state);

        // Second arguments delta
        let chunk3 = make_chunk(
            None,
            None,
            None,
            None,
            None,
            Some(vec![OpenAIDeltaToolCall {
                index: 0,
                id: None,
                call_type: None,
                function: Some(OpenAIDeltaFunction {
                    name: None,
                    arguments: Some(":\"NYC\"}".into()),
                }),
            }]),
        );
        convert_stream_chunk(&chunk3, &mut state);

        // Verify accumulated buffer
        let tc_state = state.tool_calls.get(&0).unwrap();
        assert_eq!(tc_state.arguments_buffer, "{\"location\":\"NYC\"}");
    }

    #[test]
    fn test_format_sse_all_event_types() {
        let events: Vec<AnthropicSSEEvent> = vec![
            AnthropicSSEEvent::MessageStart {
                message: AnthropicStreamMessage {
                    id: "msg_test".into(),
                    msg_type: "message".into(),
                    role: "assistant".into(),
                    content: vec![],
                    model: "gpt-4".into(),
                    stop_reason: None,
                    stop_sequence: None,
                    usage: AnthropicUsage {
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                },
            },
            AnthropicSSEEvent::ContentBlockStart {
                index: 0,
                content_block: AnthropicResponseContentBlock::Text {
                    text: String::new(),
                },
            },
            AnthropicSSEEvent::ContentBlockDelta {
                index: 0,
                delta: ContentDelta::TextDelta { text: "hi".into() },
            },
            AnthropicSSEEvent::ContentBlockStop { index: 0 },
            AnthropicSSEEvent::MessageDelta {
                delta: MessageDeltaData {
                    stop_reason: "end_turn".into(),
                    stop_sequence: None,
                },
                usage: OutputUsage { output_tokens: 5 },
            },
            AnthropicSSEEvent::MessageStop,
            AnthropicSSEEvent::Ping,
        ];

        let expected_types = [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop",
            "ping",
        ];

        for (event, expected_type) in events.iter().zip(expected_types.iter()) {
            let output = format_sse(event);
            assert!(
                output.starts_with(&format!("event: {}\n", expected_type)),
                "Expected event type '{}', got: {}",
                expected_type,
                output.lines().next().unwrap_or("")
            );
            assert!(output.ends_with("\n\n"));
        }
    }

    // Helper
    fn make_chunk(
        role: Option<&str>,
        content: Option<&str>,
        finish_reason: Option<&str>,
        model: Option<&str>,
        usage: Option<OpenAIUsage>,
        tool_calls: Option<Vec<OpenAIDeltaToolCall>>,
    ) -> OpenAISSEChunk {
        let has_choice_data =
            role.is_some() || content.is_some() || finish_reason.is_some() || tool_calls.is_some();
        OpenAISSEChunk {
            id: Some("chatcmpl-123".into()),
            model: model.map(String::from),
            choices: if has_choice_data {
                Some(vec![OpenAIDeltaChoice {
                    index: 0,
                    delta: OpenAIDelta {
                        role: role.map(String::from),
                        content: content.map(String::from),
                        tool_calls,
                    },
                    finish_reason: finish_reason.map(String::from),
                }])
            } else {
                None
            },
            usage,
        }
    }
}
