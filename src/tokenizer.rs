use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton};

use crate::protocol::TextRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenEstimate {
    pub tokenizer: String,
    pub tokens: i64,
}

pub fn estimate_request_tokens(request: &TextRequest) -> TokenEstimate {
    let text = serde_json::to_string(request).unwrap_or_else(|_| request.model.clone());
    estimate_text_tokens(&request.model, &text)
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
