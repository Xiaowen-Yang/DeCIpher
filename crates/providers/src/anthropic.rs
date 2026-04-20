//! Anthropic provider implementation (Claude API).
//!
//! Speaks the `/v1/messages` protocol with both streaming (SSE) and
//! non-streaming JSON modes. Streaming is truly incremental — events
//! are yielded as they are parsed from the byte stream, not buffered.

use crate::model_info;
use crate::retry::RetryConfig;
use crate::types::*;
use crate::{MessageStream, Provider, ProviderError, Result};

use async_trait::async_trait;
use futures::stream::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

/// Anthropic provider client.
///
/// The `model_info()` method returns metadata for the model specified at
/// construction time. If callers send requests with a different model string,
/// they should use `model_info::lookup()` directly for that model's metadata.
pub struct AnthropicProvider {
    client: Client,
    base_url: String,
    api_key: String,
    model_info: ModelInfo,
    retry_config: RetryConfig,
}

impl AnthropicProvider {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: api_key.to_string(),
            model_info: model_info::lookup(model),
            retry_config: RetryConfig::default(),
        }
    }

    /// Override the base URL (for mock testing).
    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.to_string();
        self
    }

    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.base_url)
    }

    fn build_api_request(&self, request: &MessageRequest) -> AnthropicRequest {
        let messages: Vec<AnthropicMessage> = request
            .messages
            .iter()
            .map(|m| AnthropicMessage {
                role: m.role.clone(),
                content: match &m.content {
                    MessageContent::Text(t) => AnthropicContent::Text(t.clone()),
                    MessageContent::Blocks(blocks) => {
                        AnthropicContent::Blocks(blocks.iter().map(to_api_block).collect())
                    }
                },
            })
            .collect();

        let tools: Option<Vec<AnthropicTool>> = request.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|t| AnthropicTool {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    input_schema: t
                        .input_schema
                        .clone()
                        .unwrap_or(serde_json::json!({"type": "object"})),
                })
                .collect()
        });

        AnthropicRequest {
            model: request.model.clone(),
            max_tokens: request.max_tokens,
            messages,
            stream: request.stream,
            system: request.system.clone(),
            tools,
        }
    }

    async fn send_with_retry(
        &self,
        api_request: &AnthropicRequest,
    ) -> std::result::Result<reqwest::Response, ProviderError> {
        let mut last_error = None;

        for attempt in 0..=self.retry_config.max_retries {
            if attempt > 0 {
                let delay = self.retry_config.delay_for_attempt(attempt - 1);
                tokio::time::sleep(delay).await;
            }

            let resp = self
                .client
                .post(self.messages_url())
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json")
                .json(api_request)
                .send()
                .await;

            match resp {
                Ok(r) => {
                    let status = r.status().as_u16();
                    if (200..300).contains(&status) {
                        return Ok(r);
                    }
                    if self.retry_config.is_retryable(status) {
                        let body = r.text().await.unwrap_or_default();
                        last_error = Some(ProviderError::Api {
                            status,
                            message: body,
                        });
                        continue;
                    }
                    let body = r.text().await.unwrap_or_default();
                    return Err(ProviderError::Api {
                        status,
                        message: body,
                    });
                }
                Err(e) => {
                    last_error = Some(ProviderError::Http(e));
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or(ProviderError::MaxRetries(self.retry_config.max_retries)))
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn send_message(&self, request: MessageRequest) -> Result<MessageResponse> {
        let mut api_request = self.build_api_request(&request);
        api_request.stream = false;

        let resp = self.send_with_retry(&api_request).await?;
        let body: AnthropicResponse = resp.json().await?;

        Ok(parse_response(body))
    }

    async fn stream_message(&self, request: MessageRequest) -> Result<MessageStream> {
        let mut api_request = self.build_api_request(&request);
        api_request.stream = true;

        let resp = self.send_with_retry(&api_request).await?;

        // Truly incremental: parse SSE events from the byte stream as they
        // arrive instead of buffering the entire response first.
        let byte_stream = resp.bytes_stream();
        let sse_stream = futures::stream::unfold(
            (Box::pin(byte_stream), SseParser::new()),
            |(mut bytes, mut parser)| async move {
                loop {
                    // Drain any events already parsed from previous chunks.
                    if let Some(event) = parser.next_event() {
                        return Some((event, (bytes, parser)));
                    }
                    // Read more bytes from the network.
                    match bytes.next().await {
                        Some(Ok(chunk)) => {
                            parser.feed(&chunk);
                        }
                        Some(Err(e)) => {
                            return Some((Err(ProviderError::Http(e)), (bytes, parser)));
                        }
                        None => {
                            // Stream ended — flush any trailing event.
                            return parser.finish().map(|ev| (ev, (bytes, parser)));
                        }
                    }
                }
            },
        );

        Ok(Box::pin(sse_stream))
    }

    fn model_info(&self) -> &ModelInfo {
        &self.model_info
    }
}

// ── Incremental SSE parser ─────────────────────────────────────────

/// Parses SSE frames incrementally from a byte stream.
///
/// Feed it byte chunks via `feed()`, then drain parsed events via
/// `next_event()`. Call `finish()` after the stream ends to flush any
/// trailing event that wasn't terminated by an empty line.
///
/// UTF-8 safety: raw bytes are accumulated until a valid UTF-8 string
/// boundary is found. Partial multi-byte sequences at chunk boundaries
/// are held in `raw_tail` until the next chunk completes them.
pub(crate) struct SseParser {
    /// Raw bytes that may end with an incomplete UTF-8 sequence.
    raw_tail: Vec<u8>,
    /// Decoded text that hasn't been split into lines yet.
    buffer: String,
    /// Current event name (from `event: ...` line).
    current_event: String,
    /// Current data payload (from `data: ...` line).
    current_data: String,
    /// Parsed events ready to yield.
    ready: std::collections::VecDeque<Result<StreamEvent>>,
}

impl SseParser {
    pub(crate) fn new() -> Self {
        Self {
            raw_tail: Vec::new(),
            buffer: String::new(),
            current_event: String::new(),
            current_data: String::new(),
            ready: std::collections::VecDeque::new(),
        }
    }

    /// Feed a raw byte chunk from the network. Handles multi-byte UTF-8
    /// characters that may be split across chunk boundaries.
    pub(crate) fn feed(&mut self, chunk: &[u8]) {
        self.raw_tail.extend_from_slice(chunk);

        // Find the longest valid UTF-8 prefix. Any trailing incomplete
        // multi-byte sequence stays in raw_tail for the next feed().
        let valid_up_to = match std::str::from_utf8(&self.raw_tail) {
            Ok(_) => self.raw_tail.len(),
            Err(e) => e.valid_up_to(),
        };

        if valid_up_to > 0 {
            // Safety: we just validated this slice is valid UTF-8.
            let valid =
                unsafe { std::str::from_utf8_unchecked(&self.raw_tail[..valid_up_to]) };
            self.buffer.push_str(valid);
            self.raw_tail = self.raw_tail[valid_up_to..].to_vec();
            self.parse_lines();
        }
    }

    fn parse_lines(&mut self) {
        while let Some(newline_pos) = self.buffer.find('\n') {
            let line = self.buffer[..newline_pos]
                .trim_end_matches('\r')
                .to_string();
            self.buffer = self.buffer[newline_pos + 1..].to_string();
            self.process_line(&line);
        }
    }

    fn process_line(&mut self, line: &str) {
        if line.is_empty() {
            // Empty line = event boundary.
            if !self.current_event.is_empty() && !self.current_data.is_empty() {
                let result =
                    parse_single_sse_event(&self.current_event, &self.current_data);
                match result {
                    Ok(Some(event)) => self.ready.push_back(Ok(event)),
                    Ok(None) => {}
                    Err(e) => self.ready.push_back(Err(e)),
                }
            }
            self.current_event.clear();
            self.current_data.clear();
        } else if let Some(event_name) = line.strip_prefix("event: ") {
            self.current_event = event_name.to_string();
        } else if let Some(data) = line.strip_prefix("data: ") {
            self.current_data = data.to_string();
        }
    }

    /// Pop the next parsed event, if any.
    pub(crate) fn next_event(&mut self) -> Option<Result<StreamEvent>> {
        self.ready.pop_front()
    }

    /// Flush any trailing content after the byte stream ends.
    ///
    /// Handles the case where the final `event: ..\ndata: ..\n` block
    /// was not followed by a trailing empty line.
    pub(crate) fn finish(&mut self) -> Option<Result<StreamEvent>> {
        // First, convert any remaining raw bytes (shouldn't normally
        // happen, but be defensive).
        if !self.raw_tail.is_empty() {
            let leftover = String::from_utf8_lossy(&self.raw_tail).to_string();
            self.buffer.push_str(&leftover);
            self.raw_tail.clear();
        }

        // Process any remaining complete lines in the buffer.
        self.parse_lines();

        // Drain anything parse_lines produced.
        if let Some(ev) = self.ready.pop_front() {
            return Some(ev);
        }

        // If buffer still has a non-empty unterminated line, treat it
        // as the final data/event line.
        if !self.buffer.is_empty() {
            let remaining = std::mem::take(&mut self.buffer);
            self.process_line(remaining.trim_end_matches('\r'));
        }

        // Now emit the trailing event if event+data are populated.
        if !self.current_event.is_empty() && !self.current_data.is_empty() {
            let result =
                parse_single_sse_event(&self.current_event, &self.current_data);
            self.current_event.clear();
            self.current_data.clear();
            match result {
                Ok(Some(event)) => return Some(Ok(event)),
                Ok(None) => {}
                Err(e) => return Some(Err(e)),
            }
        }
        None
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn parser_yields_events_incrementally() {
        let mut parser = SseParser::new();

        // Feed a complete SSE frame in two small chunks.
        parser.feed(b"event: message_st");
        assert!(parser.next_event().is_none(), "No event yet — line incomplete");

        parser.feed(b"op\ndata: {\"type\":\"message_stop\"}\n");
        assert!(parser.next_event().is_none(), "No event yet — no empty line");

        parser.feed(b"\n");
        let event = parser.next_event().expect("Event should be ready now");
        assert!(matches!(event, Ok(StreamEvent::MessageStop)));
        assert!(parser.next_event().is_none(), "Only one event");
    }

    #[test]
    fn parser_handles_utf8_split_across_chunks() {
        let mut parser = SseParser::new();
        // 'é' is 0xC3 0xA9 — split it across two chunks.
        let text_json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"café"}}"#;
        let frame = format!("event: content_block_delta\ndata: {text_json}\n\n");
        let bytes = frame.as_bytes();

        // Find a split point inside the 'é' character.
        let cafe_pos = bytes
            .windows(2)
            .position(|w| w == [0xC3, 0xA9])
            .expect("Should find é bytes");
        let split = cafe_pos + 1; // Split between 0xC3 and 0xA9.

        parser.feed(&bytes[..split]);
        assert!(parser.next_event().is_none());

        parser.feed(&bytes[split..]);
        let event = parser.next_event().expect("Event should parse");
        match event {
            Ok(StreamEvent::ContentBlockDelta {
                delta: ContentDelta::TextDelta { text },
                ..
            }) => {
                assert_eq!(text, "café", "UTF-8 should be intact, not corrupted");
            }
            other => panic!("Expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn finish_flushes_unterminated_event() {
        let mut parser = SseParser::new();
        // Feed event + data but NO trailing empty line.
        parser.feed(b"event: message_stop\ndata: {\"type\":\"message_stop\"}");
        assert!(parser.next_event().is_none(), "No empty line yet");

        // Stream ends — finish should flush it.
        let event = parser.finish().expect("finish should yield the event");
        assert!(matches!(event, Ok(StreamEvent::MessageStop)));
    }

    #[test]
    fn finish_flushes_unterminated_event_with_trailing_newline() {
        let mut parser = SseParser::new();
        // Feed event + data with a newline after data but no empty line.
        parser.feed(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n");
        assert!(parser.next_event().is_none());

        let event = parser.finish().expect("finish should yield the event");
        assert!(matches!(event, Ok(StreamEvent::MessageStop)));
    }

    #[test]
    fn multiple_events_across_many_feeds() {
        let mut parser = SseParser::new();
        let sse = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"test\",\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n",
            "\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n",
            "\n",
        );

        // Feed byte-by-byte.
        for &b in sse.as_bytes() {
            parser.feed(&[b]);
        }

        let ev1 = parser.next_event().expect("First event");
        assert!(matches!(ev1, Ok(StreamEvent::MessageStart { .. })));

        let ev2 = parser.next_event().expect("Second event");
        assert!(matches!(ev2, Ok(StreamEvent::MessageStop)));

        assert!(parser.next_event().is_none());
    }
}

// ── Anthropic API wire types ───────────────────────────────────────

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

fn to_api_block(block: &ContentBlock) -> AnthropicContentBlock {
    match block {
        ContentBlock::Text { text } => AnthropicContentBlock::Text { text: text.clone() },
        ContentBlock::ToolUse { id, name, input } => AnthropicContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => AnthropicContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
    }
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    id: String,
    content: Vec<AnthropicOutputBlock>,
    model: String,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicOutputBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
}

// ── Response parsing ───────────────────────────────────────────────

fn parse_response(resp: AnthropicResponse) -> MessageResponse {
    let content = resp
        .content
        .into_iter()
        .map(|b| match b {
            AnthropicOutputBlock::Text { text } => ContentBlock::Text { text },
            AnthropicOutputBlock::ToolUse { id, name, input } => {
                ContentBlock::ToolUse { id, name, input }
            }
        })
        .collect();

    let stop_reason = parse_stop_reason(resp.stop_reason.as_deref());

    MessageResponse {
        id: resp.id,
        content,
        model: resp.model,
        stop_reason,
        usage: TokenUsage {
            input_tokens: resp.usage.input_tokens,
            output_tokens: resp.usage.output_tokens,
            cache_creation_input_tokens: resp.usage.cache_creation_input_tokens,
            cache_read_input_tokens: resp.usage.cache_read_input_tokens,
        },
    }
}

fn parse_stop_reason(s: Option<&str>) -> StopReason {
    match s {
        Some("end_turn") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    }
}

// ── SSE event parsing ──────────────────────────────────────────────

fn parse_single_sse_event(event_name: &str, data: &str) -> Result<Option<StreamEvent>> {
    let json: serde_json::Value = serde_json::from_str(data)?;

    match event_name {
        "message_start" => {
            let msg = &json["message"];
            Ok(Some(StreamEvent::MessageStart {
                id: msg["id"].as_str().unwrap_or("").to_string(),
                model: msg["model"].as_str().unwrap_or("").to_string(),
                usage: parse_usage_from_json(&msg["usage"]),
            }))
        }

        "content_block_start" => {
            let index = json["index"].as_u64().unwrap_or(0) as u32;
            let block = &json["content_block"];
            let content_block = match block["type"].as_str() {
                Some("text") => ContentBlock::Text {
                    text: block["text"].as_str().unwrap_or("").to_string(),
                },
                Some("tool_use") => ContentBlock::ToolUse {
                    id: block["id"].as_str().unwrap_or("").to_string(),
                    name: block["name"].as_str().unwrap_or("").to_string(),
                    input: block["input"].clone(),
                },
                _ => return Ok(None),
            };
            Ok(Some(StreamEvent::ContentBlockStart {
                index,
                content_block,
            }))
        }

        "content_block_delta" => {
            let index = json["index"].as_u64().unwrap_or(0) as u32;
            let delta = &json["delta"];
            let content_delta = match delta["type"].as_str() {
                Some("text_delta") => ContentDelta::TextDelta {
                    text: delta["text"].as_str().unwrap_or("").to_string(),
                },
                Some("input_json_delta") => ContentDelta::InputJsonDelta {
                    partial_json: delta["partial_json"].as_str().unwrap_or("").to_string(),
                },
                _ => return Ok(None),
            };
            Ok(Some(StreamEvent::ContentBlockDelta {
                index,
                delta: content_delta,
            }))
        }

        "content_block_stop" => {
            let index = json["index"].as_u64().unwrap_or(0) as u32;
            Ok(Some(StreamEvent::ContentBlockStop { index }))
        }

        "message_delta" => {
            let delta = &json["delta"];
            let stop_reason = parse_stop_reason(delta["stop_reason"].as_str());
            let usage = parse_usage_from_json(&json["usage"]);
            Ok(Some(StreamEvent::MessageDelta { stop_reason, usage }))
        }

        "message_stop" => Ok(Some(StreamEvent::MessageStop)),

        _ => Ok(None),
    }
}

fn parse_usage_from_json(json: &serde_json::Value) -> TokenUsage {
    TokenUsage {
        input_tokens: json["input_tokens"].as_u64().unwrap_or(0) as u32,
        output_tokens: json["output_tokens"].as_u64().unwrap_or(0) as u32,
        cache_creation_input_tokens: json["cache_creation_input_tokens"].as_u64().unwrap_or(0)
            as u32,
        cache_read_input_tokens: json["cache_read_input_tokens"].as_u64().unwrap_or(0) as u32,
    }
}

// ── Tool call reassembly ───────────────────────────────────────────

/// Per-index accumulator for streaming tool calls.
struct ToolCallAccumulator {
    id: String,
    name: String,
    json_buffer: String,
}

/// Reassemble tool calls from streaming events.
///
/// Tracks per-index state so multiple tool_use blocks in a single
/// streamed turn are reassembled independently.
/// Returns a vec of (tool_id, tool_name, reassembled_input) tuples,
/// ordered by content block index.
pub fn reassemble_tool_calls(
    events: &[StreamEvent],
) -> Vec<(String, String, serde_json::Value)> {
    let mut accumulators: HashMap<u32, ToolCallAccumulator> = HashMap::new();

    for event in events {
        match event {
            StreamEvent::ContentBlockStart {
                index,
                content_block: ContentBlock::ToolUse { id, name, .. },
            } => {
                accumulators.insert(
                    *index,
                    ToolCallAccumulator {
                        id: id.clone(),
                        name: name.clone(),
                        json_buffer: String::new(),
                    },
                );
            }
            StreamEvent::ContentBlockDelta {
                index,
                delta: ContentDelta::InputJsonDelta { partial_json },
            } => {
                if let Some(acc) = accumulators.get_mut(index) {
                    acc.json_buffer.push_str(partial_json);
                }
            }
            _ => {}
        }
    }

    // Sort by index so output order matches the stream order.
    let mut indices: Vec<u32> = accumulators.keys().copied().collect();
    indices.sort_unstable();

    indices
        .into_iter()
        .filter_map(|idx| {
            let acc = accumulators.remove(&idx)?;
            let input: serde_json::Value = serde_json::from_str(&acc.json_buffer).ok()?;
            Some((acc.id, acc.name, input))
        })
        .collect()
}

/// Convenience: reassemble a single tool call (first one found).
/// Use `reassemble_tool_calls` when multi-tool streaming is expected.
pub fn reassemble_tool_call(
    events: &[StreamEvent],
) -> Option<(String, String, serde_json::Value)> {
    reassemble_tool_calls(events).into_iter().next()
}
