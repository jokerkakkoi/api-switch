use crate::anthropic::{
    AnthropicResponseContentBlock, AnthropicSSEEvent, AnthropicStreamMessage, AnthropicUsage,
    ContentDelta, MessageDeltaData, OutputUsage,
};
use crate::openai::OpenAISSEChunk;
use uuid::Uuid;

/// Tracks state across SSE chunks to produce the correct Anthropic event sequence.
pub struct StreamState {
    pub message_id: String,
    pub model: String,
    pub input_tokens: u32,
    pub content_index: u32,
    pub started: bool,
    pub content_block_open: bool,
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
            // Role delta → start content block
            if choice.delta.role.is_some() && !state.content_block_open {
                events.push(AnthropicSSEEvent::ContentBlockStart {
                    index: state.content_index,
                    content_block: AnthropicResponseContentBlock {
                        content_type: "text".to_string(),
                        text: String::new(),
                    },
                });
                state.content_block_open = true;
            }

            // Content delta
            if let Some(ref text) = choice.delta.content {
                if !state.content_block_open {
                    events.push(AnthropicSSEEvent::ContentBlockStart {
                        index: state.content_index,
                        content_block: AnthropicResponseContentBlock {
                            content_type: "text".to_string(),
                            text: String::new(),
                        },
                    });
                    state.content_block_open = true;
                }
                events.push(AnthropicSSEEvent::ContentBlockDelta {
                    index: state.content_index,
                    delta: ContentDelta {
                        delta_type: "text_delta".to_string(),
                        text: text.clone(),
                    },
                });
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
                let output_tokens =
                    chunk.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0);

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

fn map_stop_reason(reason: &str) -> String {
    match reason {
        "stop" => "end_turn".to_string(),
        "length" => "max_tokens".to_string(),
        "content_filter" => "content_filter".to_string(),
        _ => reason.to_string(),
    }
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
    use crate::openai::{OpenAIDelta, OpenAIDeltaChoice, OpenAIUsage};

    #[test]
    fn test_stream_state_new() {
        let state = StreamState::new();
        assert!(!state.started);
        assert!(!state.content_block_open);
        assert_eq!(state.content_index, 0);
    }

    #[test]
    fn test_first_chunk_with_role_emits_start_and_block_start() {
        let chunk = make_chunk(Some("assistant"), None, None, None, None);
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
        let chunk = make_chunk(None, Some("Hello"), None, None, None);
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
        let chunk = make_chunk(None, None, Some("stop"), Some("gpt-4"), Some(usage));
        let mut state = StreamState::new();
        state.started = true;
        let events = convert_stream_chunk(&chunk, &mut state);

        // Should NOT have content_block_start since we passed content=None and role=None
        let has_msg_delta = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::MessageDelta { .. }));
        let has_msg_stop = events
            .iter()
            .any(|e| matches!(e, AnthropicSSEEvent::MessageStop));
        // Since content_block_open was false and no content arrived, block_stop won't emit
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
    }

    // Helper
    fn make_chunk(
        role: Option<&str>,
        content: Option<&str>,
        finish_reason: Option<&str>,
        model: Option<&str>,
        usage: Option<OpenAIUsage>,
    ) -> OpenAISSEChunk {
        let has_choice_data =
            role.is_some() || content.is_some() || finish_reason.is_some();
        OpenAISSEChunk {
            id: Some("chatcmpl-123".into()),
            model: model.map(String::from),
            choices: if has_choice_data {
                Some(vec![OpenAIDeltaChoice {
                    index: 0,
                    delta: OpenAIDelta {
                        role: role.map(String::from),
                        content: content.map(String::from),
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
