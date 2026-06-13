use serde_json::{Map, Value};

use crate::domain::repositories::tokenizer_repository::TokenizerRepository;

/// Maximum context window (tokens) for a model id. Substring match on the model
/// family, with a conservative fallback. Tunable (roadmap §7 lists exact values as open).
pub fn max_context_for(model: &str) -> usize {
    let m = model.to_ascii_lowercase();
    if m.contains("claude") {
        200_000
    } else if m.contains("deepseek") {
        65_536
    } else if m.contains("gemini") {
        1_000_000
    } else if m.contains("gpt-4o") || m.contains("gpt-4.1") || m.contains("gpt-4-turbo") {
        128_000
    } else if m.contains("gemma") {
        8_192
    } else {
        8_192
    }
}

/// Compute the encoded-payload token budget: messages + tool schemas, vs the model window.
/// Returns None when the model/messages are absent or the tokenizer can't count messages.
pub fn compute_prompt_budget(
    tokenizer: &dyn TokenizerRepository,
    model: &str,
    payload: &Map<String, Value>,
) -> Option<PromptBudget> {
    let messages = payload.get("messages")?.as_array()?;
    let message_tokens = tokenizer.count_messages(model, messages).ok()?;
    let tools_tokens = payload
        .get("tools")
        .and_then(|tools| tokenizer.encode(model, &tools.to_string()).ok())
        .map(|ids| ids.len())
        .unwrap_or(0);
    Some(PromptBudget::new(
        message_tokens + tools_tokens,
        max_context_for(model),
    ))
}

/// Encoded-payload token budget for one model request.
#[derive(Debug, Clone, Copy)]
pub struct PromptBudget {
    pub tokens: usize,
    pub max_context: usize,
    pub ratio: f64,
}

impl PromptBudget {
    pub fn new(tokens: usize, max_context: usize) -> Self {
        let ratio = if max_context == 0 {
            0.0
        } else {
            tokens as f64 / max_context as f64
        };
        Self {
            tokens,
            max_context,
            ratio,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_context_known_families_and_fallback() {
        assert_eq!(max_context_for("claude-opus-4-8"), 200_000);
        assert_eq!(max_context_for("deepseek-chat"), 65_536);
        assert_eq!(max_context_for("gemini-2.5-pro"), 1_000_000);
        assert_eq!(max_context_for("gpt-4o-mini"), 128_000);
        assert_eq!(max_context_for("totally-unknown-model"), 8_192);
    }

    #[test]
    fn prompt_budget_ratio_math() {
        let b = PromptBudget::new(64_000, 128_000);
        assert_eq!(b.tokens, 64_000);
        assert_eq!(b.max_context, 128_000);
        assert!((b.ratio - 0.5).abs() < 1e-9);
        assert_eq!(PromptBudget::new(10, 0).ratio, 0.0);
    }
}
