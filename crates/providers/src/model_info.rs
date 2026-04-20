use crate::types::ModelInfo;

/// Look up model metadata by model ID string.
///
/// Returns metadata for known model families. Unknown models get
/// reasonable defaults (128k context, tools enabled) that match the
/// JS runtime behavior in `lib/compact.js` rather than being overly
/// conservative — a provider wouldn't be called if tools weren't
/// supported, and most modern models have large context windows.
pub fn lookup(model: &str) -> ModelInfo {
    // Anthropic models
    if model.starts_with("claude-opus-4") {
        return ModelInfo {
            id: model.to_string(),
            context_window: 200_000,
            max_output_tokens: 32_000,
            supports_tools: true,
            supports_streaming: true,
        };
    }
    if model.starts_with("claude-sonnet-4") {
        return ModelInfo {
            id: model.to_string(),
            context_window: 200_000,
            max_output_tokens: 16_000,
            supports_tools: true,
            supports_streaming: true,
        };
    }
    if model.starts_with("claude-haiku-4") || model.starts_with("claude-haiku-3") {
        return ModelInfo {
            id: model.to_string(),
            context_window: 200_000,
            max_output_tokens: 8_192,
            supports_tools: true,
            supports_streaming: true,
        };
    }
    // Catch-all for any claude model we don't have a specific match for.
    if model.starts_with("claude-") {
        return ModelInfo {
            id: model.to_string(),
            context_window: 200_000,
            max_output_tokens: 8_192,
            supports_tools: true,
            supports_streaming: true,
        };
    }

    // OpenAI models
    if model.starts_with("gpt-4o") {
        return ModelInfo {
            id: model.to_string(),
            context_window: 128_000,
            max_output_tokens: 16_384,
            supports_tools: true,
            supports_streaming: true,
        };
    }
    if model.starts_with("o1") || model.starts_with("o3") || model.starts_with("o4") {
        return ModelInfo {
            id: model.to_string(),
            context_window: 200_000,
            max_output_tokens: 100_000,
            supports_tools: true,
            supports_streaming: true,
        };
    }
    if model.starts_with("gpt-4") || model.starts_with("gpt-3") {
        return ModelInfo {
            id: model.to_string(),
            context_window: 128_000,
            max_output_tokens: 8_192,
            supports_tools: true,
            supports_streaming: true,
        };
    }

    // Unknown model — match JS runtime defaults (128k context, tools enabled).
    // Being too conservative here causes incorrect compaction timing and
    // disables tool-calling for models that actually support it.
    ModelInfo {
        id: model.to_string(),
        context_window: 128_000,
        max_output_tokens: 8_192,
        supports_tools: true,
        supports_streaming: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_models() {
        let info = lookup("claude-sonnet-4-5-20250514");
        assert_eq!(info.context_window, 200_000);
        assert!(info.supports_tools);

        let info = lookup("claude-opus-4-5-20250514");
        assert_eq!(info.max_output_tokens, 32_000);
    }

    #[test]
    fn anthropic_catch_all() {
        let info = lookup("claude-3-opus-20240229");
        assert_eq!(info.context_window, 200_000);
        assert!(info.supports_tools);
    }

    #[test]
    fn openai_models() {
        let info = lookup("gpt-4o-2024-08-06");
        assert_eq!(info.context_window, 128_000);
        assert!(info.supports_tools);
    }

    #[test]
    fn unknown_model_gets_reasonable_defaults() {
        let info = lookup("some-custom-model");
        assert_eq!(info.context_window, 128_000);
        assert!(info.supports_tools);
    }
}
