use crate::sse;
use crate::types::{InputContent, InputContentBlock, MessageRequest, MessageResponse, Usage};

pub const SCENARIO_PREFIX: &str = "PARITY_SCENARIO:";

/// Known parity scenarios. Each variant maps to a scripted response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// Phase 1: basic SSE text streaming, no tools.
    StreamingText,
    /// Phase 1: single tool_use → tool_result → final text.
    ToolCallRoundtrip,
    /// Phase 1: two tool_use blocks in one assistant response.
    MultiToolTurn,
    /// Phase 1: token usage fields in response.
    TokenUsageReporting,
    /// Phase 1: tool_use streamed via input_json_delta chunks.
    StreamingToolCall,
}

impl Scenario {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "streaming_text" => Some(Self::StreamingText),
            "tool_call_roundtrip" => Some(Self::ToolCallRoundtrip),
            "multi_tool_turn" => Some(Self::MultiToolTurn),
            "token_usage_reporting" => Some(Self::TokenUsageReporting),
            "streaming_tool_call" => Some(Self::StreamingToolCall),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::StreamingText => "streaming_text",
            Self::ToolCallRoundtrip => "tool_call_roundtrip",
            Self::MultiToolTurn => "multi_tool_turn",
            Self::TokenUsageReporting => "token_usage_reporting",
            Self::StreamingToolCall => "streaming_tool_call",
        }
    }
}

/// Detect scenario from message content by searching for `PARITY_SCENARIO:name`.
pub fn detect_scenario(request: &MessageRequest) -> Option<Scenario> {
    for msg in request.messages.iter().rev() {
        let texts = extract_texts(&msg.content);
        for text in texts {
            for token in text.split_whitespace() {
                if let Some(name) = token.strip_prefix(SCENARIO_PREFIX) {
                    return Scenario::parse(name);
                }
            }
        }
    }
    None
}

fn extract_texts(content: &InputContent) -> Vec<&str> {
    match content {
        InputContent::Text(t) => vec![t.as_str()],
        InputContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                InputContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect(),
    }
}

/// Build a non-streaming JSON response for the given scenario.
pub fn build_response(request: &MessageRequest, scenario: Scenario) -> MessageResponse {
    let has_tool_results = request.latest_tool_result().is_some();

    match scenario {
        Scenario::StreamingText => {
            // Should not be called for streaming, but handle gracefully
            MessageResponse::text("msg_streaming_text", "Hello from the mock provider! This is a streaming text test response.")
        }

        Scenario::ToolCallRoundtrip => {
            if has_tool_results {
                let result_text = request.latest_tool_result().unwrap_or_default();
                MessageResponse::text(
                    "msg_tool_roundtrip_final",
                    &format!("tool_call_roundtrip complete: {result_text}"),
                )
            } else {
                MessageResponse::tool_use(
                    "msg_tool_roundtrip",
                    "toolu_read_fixture",
                    "read_file",
                    serde_json::json!({"path": "fixture.txt"}),
                )
            }
        }

        Scenario::MultiToolTurn => {
            let results = request.tool_results_by_id();
            if results.len() >= 2 {
                let combined: String = results.iter().map(|(_, text, _)| text.as_str()).collect::<Vec<_>>().join(" + ");
                MessageResponse::text(
                    "msg_multi_tool_final",
                    &format!("multi_tool_turn complete: {combined}"),
                )
            } else {
                MessageResponse::multi_tool_use(
                    "msg_multi_tool",
                    vec![
                        (
                            "toolu_read_a".to_string(),
                            "read_file".to_string(),
                            serde_json::json!({"path": "a.txt"}),
                        ),
                        (
                            "toolu_read_b".to_string(),
                            "read_file".to_string(),
                            serde_json::json!({"path": "b.txt"}),
                        ),
                    ],
                )
            }
        }

        Scenario::TokenUsageReporting => {
            let mut resp = MessageResponse::text(
                "msg_token_usage",
                "Token usage reporting test.",
            );
            resp.usage = Usage {
                input_tokens: 142,
                output_tokens: 37,
                cache_creation_input_tokens: 10,
                cache_read_input_tokens: 5,
            };
            resp
        }

        Scenario::StreamingToolCall => {
            // Non-streaming fallback — real test uses streaming path
            if has_tool_results {
                let result_text = request.latest_tool_result().unwrap_or_default();
                MessageResponse::text(
                    "msg_streaming_tool_final",
                    &format!("streaming_tool_call complete: {result_text}"),
                )
            } else {
                MessageResponse::tool_use(
                    "msg_streaming_tool",
                    "toolu_grep_search",
                    "grep_search",
                    serde_json::json!({"pattern": "parity", "path": "fixture.txt"}),
                )
            }
        }
    }
}

/// Build an SSE stream body for the given scenario.
pub fn build_stream(request: &MessageRequest, scenario: Scenario) -> String {
    let usage = Usage::default();
    let has_tool_results = request.latest_tool_result().is_some();

    match scenario {
        Scenario::StreamingText => {
            let usage = Usage::new(80, 35);
            sse::streaming_text(
                "Hello from the mock provider! This is a streaming text test response.",
                &usage,
            )
        }

        Scenario::ToolCallRoundtrip => {
            if has_tool_results {
                let result_text = request.latest_tool_result().unwrap_or_default();
                sse::streaming_final_text(
                    &format!("tool_call_roundtrip complete: {result_text}"),
                    &usage,
                )
            } else {
                sse::streaming_tool_use(
                    "toolu_read_fixture",
                    "read_file",
                    r#"{"path":"fixture.txt"}"#,
                    &usage,
                )
            }
        }

        Scenario::MultiToolTurn => {
            let results = request.tool_results_by_id();
            if results.len() >= 2 {
                let combined: String = results.iter().map(|(_, text, _)| text.as_str()).collect::<Vec<_>>().join(" + ");
                sse::streaming_final_text(
                    &format!("multi_tool_turn complete: {combined}"),
                    &usage,
                )
            } else {
                sse::streaming_multi_tool(
                    &[
                        ("toolu_read_a", "read_file", r#"{"path":"a.txt"}"#),
                        ("toolu_read_b", "read_file", r#"{"path":"b.txt"}"#),
                    ],
                    &usage,
                )
            }
        }

        Scenario::TokenUsageReporting => {
            let usage = Usage {
                input_tokens: 142,
                output_tokens: 37,
                cache_creation_input_tokens: 10,
                cache_read_input_tokens: 5,
            };
            sse::streaming_text("Token usage reporting test.", &usage)
        }

        Scenario::StreamingToolCall => {
            if has_tool_results {
                let result_text = request.latest_tool_result().unwrap_or_default();
                sse::streaming_final_text(
                    &format!("streaming_tool_call complete: {result_text}"),
                    &usage,
                )
            } else {
                sse::streaming_tool_use(
                    "toolu_grep_search",
                    "grep_search",
                    r#"{"pattern":"parity","path":"fixture.txt"}"#,
                    &usage,
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_scenarios() {
        assert_eq!(Scenario::parse("streaming_text"), Some(Scenario::StreamingText));
        assert_eq!(Scenario::parse("tool_call_roundtrip"), Some(Scenario::ToolCallRoundtrip));
        assert_eq!(Scenario::parse("multi_tool_turn"), Some(Scenario::MultiToolTurn));
        assert_eq!(Scenario::parse("token_usage_reporting"), Some(Scenario::TokenUsageReporting));
        assert_eq!(Scenario::parse("streaming_tool_call"), Some(Scenario::StreamingToolCall));
        assert_eq!(Scenario::parse("unknown"), None);
    }

    #[test]
    fn detect_scenario_from_request() {
        let request = MessageRequest {
            model: "test".to_string(),
            max_tokens: 1024,
            messages: vec![crate::types::InputMessage {
                role: "user".to_string(),
                content: InputContent::Text(
                    "PARITY_SCENARIO:tool_call_roundtrip please read fixture.txt".to_string(),
                ),
            }],
            stream: false,
            system: None,
            tools: None,
        };
        assert_eq!(detect_scenario(&request), Some(Scenario::ToolCallRoundtrip));
    }

    #[test]
    fn roundtrip_initial_returns_tool_use() {
        let request = MessageRequest {
            model: "test".to_string(),
            max_tokens: 1024,
            messages: vec![crate::types::InputMessage {
                role: "user".to_string(),
                content: InputContent::Text("PARITY_SCENARIO:tool_call_roundtrip".to_string()),
            }],
            stream: false,
            system: None,
            tools: None,
        };
        let resp = build_response(&request, Scenario::ToolCallRoundtrip);
        assert_eq!(resp.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(resp.content.len(), 1);
    }
}
