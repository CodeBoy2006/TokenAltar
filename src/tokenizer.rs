use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton};

use crate::protocol::{MessagePart, TextRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenEstimate {
    pub tokenizer: String,
    pub tokens: i64,
}

pub fn estimate_request_tokens(request: &TextRequest) -> TokenEstimate {
    let sanitized = sanitize_request_for_token_estimate(request);
    let text = serde_json::to_string(&sanitized).unwrap_or_else(|_| request.model.clone());
    let mut estimate = estimate_text_tokens(&request.model, &text);
    estimate.tokens += count_image_parts(request) * 512;
    estimate
}

pub fn estimate_text_tokens(model: &str, text: &str) -> TokenEstimate {
    let (tokenizer, count) = if model.starts_with("gpt-4o")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
    {
        (
            "o200k_base",
            o200k_base_singleton()
                .encode_with_special_tokens(text)
                .len(),
        )
    } else if model.starts_with("claude") {
        // Anthropic does not expose a local Rust tokenizer. cl100k is used as a deterministic
        // conservative precheck proxy; actual settlement still comes from upstream usage.
        (
            "cl100k_base_proxy_for_anthropic",
            cl100k_base_singleton()
                .encode_with_special_tokens(text)
                .len(),
        )
    } else {
        (
            "cl100k_base",
            cl100k_base_singleton()
                .encode_with_special_tokens(text)
                .len(),
        )
    };
    TokenEstimate {
        tokenizer: tokenizer.to_string(),
        tokens: count.max(1) as i64,
    }
}

fn sanitize_request_for_token_estimate(request: &TextRequest) -> TextRequest {
    let mut sanitized = request.clone();
    for message in &mut sanitized.messages {
        for part in &mut message.content {
            if matches!(part, MessagePart::Image(_)) {
                *part = MessagePart::Text("[image]".to_string());
            }
        }
    }
    sanitized
}

fn count_image_parts(request: &TextRequest) -> i64 {
    request
        .messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter(|part| matches!(part, MessagePart::Image(_)))
        .count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_with_named_tokenizer() {
        let estimate = estimate_text_tokens("gpt-4o-mini", "hello world");
        assert_eq!(estimate.tokenizer, "o200k_base");
        assert!(estimate.tokens > 0);
    }
}
