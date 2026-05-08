use crate::anthropic::{
    AnthropicContent, AnthropicContentBlock, AnthropicRequest, AnthropicSystem,
};
use crate::openai::{ImageUrl, OpenAIContent, OpenAIContentBlock, OpenAIMessage, OpenAIRequest};

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
        });
    }

    // Convert messages
    for msg in req.messages {
        let content = convert_content(msg.content);
        messages.push(OpenAIMessage {
            role: msg.role,
            content,
        });
    }

    OpenAIRequest {
        model: req.model,
        messages,
        max_tokens: Some(req.max_tokens),
        stop: req.stop_sequences,
        temperature: req.temperature,
        top_p: req.top_p,
        stream: req.stream,
    }
}

fn convert_content(content: AnthropicContent) -> OpenAIContent {
    match content {
        AnthropicContent::SingleString(text) => OpenAIContent::Text(text),
        AnthropicContent::TextBlocks(blocks) => {
            // If single text block, use simple Text variant
            if blocks.len() == 1 && blocks[0].content_type == "text" {
                if let Some(ref text) = blocks[0].text {
                    return OpenAIContent::Text(text.clone());
                }
            }
            // Multi-block or image → convert each block
            let converted: Vec<OpenAIContentBlock> = blocks
                .into_iter()
                .map(convert_content_block)
                .collect();
            OpenAIContent::MultiContent(converted)
        }
    }
}

fn convert_content_block(block: AnthropicContentBlock) -> OpenAIContentBlock {
    match block.content_type.as_str() {
        "image" => {
            let url = block
                .source
                .map(|src| format!("data:{};base64,{}", src.media_type, src.data))
                .unwrap_or_default();
            OpenAIContentBlock {
                content_type: "image_url".to_string(),
                text: None,
                image_url: Some(ImageUrl { url }),
            }
        }
        _ => OpenAIContentBlock {
            content_type: "text".to_string(),
            text: block.text,
            image_url: None,
        },
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
}
