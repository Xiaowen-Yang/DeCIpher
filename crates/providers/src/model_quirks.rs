//! Model-specific behavioral quirks for LLM API clients.
//!
//! This module captures three distinct concerns:
//!
//! 1. **Parameter rejection** (`is_reasoning_model`): reasoning/chain-of-thought
//!    models (o1, o3, o4, grok-3-mini, qwq, deepseek-thinking) reject standard
//!    sampling parameters such as `temperature`, `top_p`, `frequency_penalty`,
//!    and `presence_penalty`. Callers must omit those fields entirely.
//!
//! 2. **Output token field naming** (`max_tokens_field_name`): OpenAI reasoning
//!    models (o1/o3/o4) and GPT-5 use `max_completion_tokens` rather than the
//!    conventional `max_tokens` field in their request bodies.
//!
//! 3. **Thinking-mode support** (`supports_thinking_mode`): a small subset of
//!    models (currently GLM) emit structured thinking/reasoning output blocks
//!    in their responses and accept a `thinking` configuration object.

/// Returns true for models that reject temperature, top_p, frequency_penalty,
/// and presence_penalty.
pub fn is_reasoning_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m == "o1" || m.starts_with("o1-")
        || m == "o3" || m.starts_with("o3-")
        || m == "o4" || m.starts_with("o4-")
        || m.starts_with("grok-3-mini")
        || m.starts_with("qwen-qwq")
        || m.starts_with("qwq")
        || m.contains("thinking")
}

/// Returns the correct JSON field name for the output token limit.
/// OpenAI reasoning models (o1/o3/o4) and GPT-5 use `max_completion_tokens`.
pub fn max_tokens_field_name(model: &str) -> &'static str {
    let m = model.to_lowercase();
    if m == "o1" || m.starts_with("o1-")
        || m == "o3" || m.starts_with("o3-")
        || m == "o4" || m.starts_with("o4-")
        || m == "gpt-5" || m.starts_with("gpt-5-")
    {
        "max_completion_tokens"
    } else {
        "max_tokens"
    }
}

/// Returns true for models that support thinking/reasoning output blocks.
pub fn supports_thinking_mode(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("glm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_model_positives() {
        assert!(is_reasoning_model("o1"));
        assert!(is_reasoning_model("o1-mini"));
        assert!(is_reasoning_model("o1-preview"));
        assert!(is_reasoning_model("o3"));
        assert!(is_reasoning_model("o3-mini"));
        assert!(is_reasoning_model("o4-mini"));
        assert!(is_reasoning_model("grok-3-mini"));
        assert!(is_reasoning_model("qwen-qwq-32b"));
        assert!(is_reasoning_model("qwq-32b"));
        assert!(is_reasoning_model("deepseek-thinking"));
        assert!(is_reasoning_model("some-thinking-model"));
    }

    #[test]
    fn reasoning_model_negatives() {
        assert!(!is_reasoning_model("gpt-4o"));
        assert!(!is_reasoning_model("gpt-4o-mini"));
        assert!(!is_reasoning_model("claude-sonnet-4-6"));
        assert!(!is_reasoning_model("claude-opus-4-5"));
        assert!(!is_reasoning_model("glm-5.1"));
        assert!(!is_reasoning_model("grok-3"));
    }

    #[test]
    fn reasoning_model_case_insensitive() {
        assert!(is_reasoning_model("O3-Mini"));
        assert!(is_reasoning_model("O1-Preview"));
        assert!(is_reasoning_model("QWQ-32B"));
    }

    #[test]
    fn max_tokens_field_reasoning_models() {
        assert_eq!(max_tokens_field_name("o1"), "max_completion_tokens");
        assert_eq!(max_tokens_field_name("o3-mini"), "max_completion_tokens");
        assert_eq!(max_tokens_field_name("o4-mini"), "max_completion_tokens");
        assert_eq!(max_tokens_field_name("gpt-5"), "max_completion_tokens");
        assert_eq!(max_tokens_field_name("gpt-5-turbo"), "max_completion_tokens");
    }

    #[test]
    fn max_tokens_field_standard_models() {
        assert_eq!(max_tokens_field_name("gpt-4o"), "max_tokens");
        assert_eq!(max_tokens_field_name("gpt-4o-mini"), "max_tokens");
        assert_eq!(max_tokens_field_name("claude-sonnet-4-6"), "max_tokens");
        assert_eq!(max_tokens_field_name("glm-5.1"), "max_tokens");
        assert_eq!(max_tokens_field_name("qwq-32b"), "max_tokens");
    }

    #[test]
    fn max_tokens_field_case_insensitive() {
        assert_eq!(max_tokens_field_name("O3-Mini"), "max_completion_tokens");
        assert_eq!(max_tokens_field_name("GPT-5"), "max_completion_tokens");
    }

    #[test]
    fn thinking_mode_positives() {
        assert!(supports_thinking_mode("glm-5.1"));
        assert!(supports_thinking_mode("glm-4-plus"));
        assert!(supports_thinking_mode("GLM-5.1"));
    }

    #[test]
    fn thinking_mode_negatives() {
        assert!(!supports_thinking_mode("gpt-4o"));
        assert!(!supports_thinking_mode("claude-sonnet-4-6"));
        assert!(!supports_thinking_mode("o3-mini"));
        assert!(!supports_thinking_mode("qwq-32b"));
    }
}
