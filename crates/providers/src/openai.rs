//! OpenAI-compatible provider implementation.
//!
//! Speaks the `/v1/chat/completions` protocol used by OpenAI, ZhiPu (GLM),
//! Groq, Mistral, Together, Deepseek, and most other LLM APIs.
//!
//! Translates between DeCIpher's internal message types (Anthropic-style
//! ContentBlocks) and OpenAI's chat completions wire format.

use crate::model_info;
use crate::retry::RetryConfig;
use crate::types::*;
use crate::{MessageStream, Provider, ProviderError, Result};

use async_trait::async_trait;
use futures::stream::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// OpenAI-compatible provider client.
///
/// Works with any API that implements the `/v1/chat/completions` protocol:
/// OpenAI, ZhiPu/GLM, Groq, Mistral, Together, Deepseek, vLLM, etc.
pub struct OpenAiProvider {
    client: Client,
    base_url: String,
    api_key: String,
    model_info: ModelInfo,
    retry_config: RetryConfig,
}

impl OpenAiProvider {
    pub fn new(api_key: &str, model: &str, base_url: &str) -> Self {
        // Normalize: strip trailing slashes.
        let base = base_url.trim_end_matches('/').to_string();
        Self {
            client: Client::new(),
            base_url: base,
            api_key: api_key.to_string(),
            model_info: model_info::lookup(model),
            retry_config: RetryConfig::default(),
        }
    }

    fn completions_url(&self) -> String {
        // If the base URL already ends with a versioned API path (e.g. /v4, /v1),
        // append /chat/completions directly. Otherwise prepend /v1/.
        // This handles ZhiPu (/api/paas/v4) and OpenAI (/v1) correctly.
        if self.base_url.ends_with("/v1") {
            format!("{}/chat/completions", self.base_url)
        } else if self.base_url.contains("/v") {
            // Versioned path like /api/paas/v4 — append directly.
            format!("{}/chat/completions", self.base_url)
        } else {
            format!("{}/v1/chat/completions", self.base_url)
        }
    }

    /// Convert DeCIpher internal types → OpenAI request format.
    fn build_request(&self, request: &MessageRequest) -> OpenAiRequest {
        let mut messages: Vec<OpenAiMessage> = Vec::new();

        // System prompt → system message.
        if let Some(ref sys) = request.system {
            if !sys.is_empty() {
                messages.push(OpenAiMessage {
                    role: "system".to_string(),
                    content: Some(sys.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                });
            }
        }

        // Convert each DeCIpher message.
        for msg in &request.messages {
            match &msg.content {
                MessageContent::Text(text) => {
                    messages.push(OpenAiMessage {
                        role: msg.role.clone(),
                        content: Some(text.clone()),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    });
                }
                MessageContent::Blocks(blocks) => {
                    // Blocks can contain Text + ToolUse (assistant) or ToolResult (user).
                    let mut text_parts: Vec<String> = Vec::new();
                    let mut tool_calls: Vec<OpenAiToolCall> = Vec::new();
                    let mut tool_results: Vec<OpenAiMessage> = Vec::new();

                    for block in blocks {
                        match block {
                            ContentBlock::Text { text } => {
                                text_parts.push(text.clone());
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                tool_calls.push(OpenAiToolCall {
                                    id: id.clone(),
                                    r#type: "function".to_string(),
                                    function: OpenAiFunctionCall {
                                        name: name.clone(),
                                        arguments: serde_json::to_string(input)
                                            .unwrap_or_default(),
                                    },
                                });
                            }
                            ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                ..
                            } => {
                                tool_results.push(OpenAiMessage {
                                    role: "tool".to_string(),
                                    content: Some(content.clone()),
                                    tool_calls: None,
                                    tool_call_id: Some(tool_use_id.clone()),
                                    name: None,
                                });
                            }
                        }
                    }

                    // Emit assistant message with text + tool_calls.
                    if !text_parts.is_empty() || !tool_calls.is_empty() {
                        messages.push(OpenAiMessage {
                            role: msg.role.clone(),
                            content: if text_parts.is_empty() {
                                None
                            } else {
                                Some(text_parts.join("\n"))
                            },
                            tool_calls: if tool_calls.is_empty() {
                                None
                            } else {
                                Some(tool_calls)
                            },
                            tool_call_id: None,
                            name: None,
                        });
                    }

                    // Emit tool result messages (each is a separate message).
                    messages.extend(tool_results);
                }
            }
        }

        // Convert tools to OpenAI format.
        let tools: Option<Vec<OpenAiToolDef>> = request.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|t| OpenAiToolDef {
                    r#type: "function".to_string(),
                    function: OpenAiFunctionDef {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        parameters: t
                            .input_schema
                            .clone()
                            .unwrap_or(serde_json::json!({"type": "object"})),
                    },
                })
                .collect()
        });

        OpenAiRequest {
            model: request.model.clone(),
            messages,
            tools,
            max_tokens: Some(request.max_tokens),
            stream: request.stream,
            stream_options: if request.stream {
                Some(StreamOptions { include_usage: true })
            } else {
                None
            },
        }
    }

    async fn send_with_retry(
        &self,
        api_request: &OpenAiRequest,
    ) -> std::result::Result<reqwest::Response, ProviderError> {
        let mut last_error = None;

        for attempt in 0..=self.retry_config.max_retries {
            if attempt > 0 {
                let delay = self.retry_config.delay_for_attempt(attempt - 1);
                tokio::time::sleep(delay).await;
            }

            let resp = self
                .client
                .post(self.completions_url())
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
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

impl std::fmt::Debug for OpenAiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiProvider")
            .field("base_url", &self.base_url)
            .field("model", &self.model_info.id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    async fn send_message(&self, request: MessageRequest) -> Result<MessageResponse> {
        let mut api_request = self.build_request(&request);
        api_request.stream = false;
        api_request.stream_options = None;

        let resp = self.send_with_retry(&api_request).await?;
        let body: OpenAiResponse = resp.json().await?;

        Ok(parse_response(body))
    }

    async fn stream_message(&self, request: MessageRequest) -> Result<MessageStream> {
        let mut api_request = self.build_request(&request);
        api_request.stream = true;

        let resp = self.send_with_retry(&api_request).await?;

        let byte_stream = resp.bytes_stream();
        let sse_stream = futures::stream::unfold(
            (Box::pin(byte_stream), OpenAiSseParser::new()),
            |(mut bytes, mut parser)| async move {
                loop {
                    if let Some(event) = parser.next_event() {
                        return Some((event, (bytes, parser)));
                    }
                    match bytes.next().await {
                        Some(Ok(chunk)) => {
                            parser.feed(&chunk);
                        }
                        Some(Err(e)) => {
                            return Some((Err(ProviderError::Http(e)), (bytes, parser)));
                        }
                        None => {
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

// ── OpenAI wire types ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
}

#[derive(Debug, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    r#type: String,
    function: OpenAiFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OpenAiToolDef {
    r#type: String,
    function: OpenAiFunctionDef,
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionDef {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: serde_json::Value,
}

// ── Response types ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    id: String,
    model: String,
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

// ── Streaming chunk types ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OpenAiStreamChunk {
    id: String,
    model: String,
    choices: Vec<OpenAiStreamChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    #[serde(default)]
    #[allow(dead_code)]
    index: u32,
    delta: OpenAiDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiDelta {
    #[serde(default)]
    #[allow(dead_code)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiDeltaToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiDeltaToolCall {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<OpenAiDeltaFunction>,
}

#[derive(Debug, Deserialize)]
struct OpenAiDeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

// ── Non-streaming response parsing ───────────────────────────────────

fn parse_response(resp: OpenAiResponse) -> MessageResponse {
    let usage = resp.usage.unwrap_or_default();
    let choice = match resp.choices.into_iter().next() {
        Some(c) => c,
        None => {
            return MessageResponse {
                id: resp.id,
                content: vec![],
                model: resp.model,
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage {
                    input_tokens: usage.prompt_tokens,
                    output_tokens: usage.completion_tokens,
                    ..Default::default()
                },
            };
        }
    };

    let mut content: Vec<ContentBlock> = Vec::new();

    if let Some(text) = choice.message.content {
        if !text.is_empty() {
            content.push(ContentBlock::Text { text });
        }
    }

    if let Some(tool_calls) = choice.message.tool_calls {
        for tc in tool_calls {
            let input: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::json!({}));
            content.push(ContentBlock::ToolUse {
                id: tc.id,
                name: tc.function.name,
                input,
            });
        }
    }

    let stop_reason = match choice.finish_reason.as_deref() {
        Some("tool_calls") => StopReason::ToolUse,
        Some("length") => StopReason::MaxTokens,
        Some("stop") => StopReason::EndTurn,
        _ => StopReason::EndTurn,
    };

    MessageResponse {
        id: resp.id,
        content,
        model: resp.model,
        stop_reason,
        usage: TokenUsage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            ..Default::default()
        },
    }
}

// ── SSE stream parser ────────────────────────────────────────────────

/// Incremental SSE parser for OpenAI streaming format.
///
/// Translates OpenAI's streaming chunks into DeCIpher's StreamEvent types.
/// Tracks per-index tool call state for reassembly.
struct OpenAiSseParser {
    raw_tail: Vec<u8>,
    buffer: String,
    ready: std::collections::VecDeque<Result<StreamEvent>>,
    /// Whether we've emitted a MessageStart event.
    started: bool,
    /// Current text content block index (for text deltas).
    text_block_index: Option<u32>,
    /// Tool call accumulators: delta index → (call_id, name, json_buffer, block_index).
    tool_accumulators: HashMap<u32, (String, String, String, u32)>,
    /// Next content block index to allocate.
    next_block_index: u32,
    /// Saved model/id from first chunk.
    stream_id: String,
    stream_model: String,
}

impl OpenAiSseParser {
    fn new() -> Self {
        Self {
            raw_tail: Vec::new(),
            buffer: String::new(),
            ready: std::collections::VecDeque::new(),
            started: false,
            text_block_index: None,
            tool_accumulators: HashMap::new(),
            next_block_index: 0,
            stream_id: String::new(),
            stream_model: String::new(),
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        self.raw_tail.extend_from_slice(chunk);
        let valid_up_to = match std::str::from_utf8(&self.raw_tail) {
            Ok(_) => self.raw_tail.len(),
            Err(e) => e.valid_up_to(),
        };
        if valid_up_to > 0 {
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
        let line = line.trim();
        if line.is_empty() || line.starts_with(':') {
            return;
        }
        if let Some(data) = line.strip_prefix("data: ") {
            let data = data.trim();
            if data == "[DONE]" {
                self.emit_stream_end();
                return;
            }
            self.process_data(data);
        }
    }

    fn process_data(&mut self, data: &str) {
        let chunk: OpenAiStreamChunk = match serde_json::from_str(data) {
            Ok(c) => c,
            Err(e) => {
                self.ready
                    .push_back(Err(ProviderError::SseParse(format!("JSON parse: {e}"))));
                return;
            }
        };

        // Emit MessageStart on first chunk.
        if !self.started {
            self.stream_id = chunk.id.clone();
            self.stream_model = chunk.model.clone();
            let usage = chunk.usage.as_ref().map_or(TokenUsage::default(), |u| {
                TokenUsage {
                    input_tokens: u.prompt_tokens,
                    output_tokens: u.completion_tokens,
                    ..Default::default()
                }
            });
            self.ready.push_back(Ok(StreamEvent::MessageStart {
                id: chunk.id.clone(),
                model: chunk.model.clone(),
                usage,
            }));
            self.started = true;
        }

        // Process usage update (from stream_options.include_usage).
        if let Some(ref usage) = chunk.usage {
            if usage.prompt_tokens > 0 || usage.completion_tokens > 0 {
                self.ready.push_back(Ok(StreamEvent::MessageDelta {
                    stop_reason: StopReason::EndTurn,
                    usage: TokenUsage {
                        input_tokens: usage.prompt_tokens,
                        output_tokens: usage.completion_tokens,
                        ..Default::default()
                    },
                }));
            }
        }

        for choice in &chunk.choices {
            // Handle text content delta.
            if let Some(ref text) = choice.delta.content {
                if !text.is_empty() {
                    let idx = *self.text_block_index.get_or_insert_with(|| {
                        let idx = self.next_block_index;
                        self.next_block_index += 1;
                        self.ready.push_back(Ok(StreamEvent::ContentBlockStart {
                            index: idx,
                            content_block: ContentBlock::Text {
                                text: String::new(),
                            },
                        }));
                        idx
                    });
                    self.ready.push_back(Ok(StreamEvent::ContentBlockDelta {
                        index: idx,
                        delta: ContentDelta::TextDelta {
                            text: text.clone(),
                        },
                    }));
                }
            }

            // Handle tool call deltas.
            if let Some(ref tool_calls) = choice.delta.tool_calls {
                // Close text block if we're transitioning to tool calls.
                if let Some(text_idx) = self.text_block_index.take() {
                    self.ready
                        .push_back(Ok(StreamEvent::ContentBlockStop { index: text_idx }));
                }

                for tc_delta in tool_calls {
                    let tc_index = tc_delta.index;

                    // Ensure accumulator exists for this tool call index.
                    if !self.tool_accumulators.contains_key(&tc_index) {
                        let block_idx = self.next_block_index;
                        self.next_block_index += 1;
                        self.tool_accumulators.insert(
                            tc_index,
                            (String::new(), String::new(), String::new(), block_idx),
                        );
                    }
                    let acc = self.tool_accumulators.get_mut(&tc_index).unwrap();

                    if let Some(ref id) = tc_delta.id {
                        acc.0 = id.clone();
                    }
                    if let Some(ref func) = tc_delta.function {
                        if let Some(ref name) = func.name {
                            if acc.1.is_empty() {
                                acc.1 = name.clone();
                                // Now we have id + name: emit ContentBlockStart.
                                self.ready.push_back(Ok(StreamEvent::ContentBlockStart {
                                    index: acc.3,
                                    content_block: ContentBlock::ToolUse {
                                        id: acc.0.clone(),
                                        name: name.clone(),
                                        input: serde_json::json!({}),
                                    },
                                }));
                            }
                        }
                        if let Some(ref args) = func.arguments {
                            if !args.is_empty() {
                                acc.2.push_str(args);
                                self.ready.push_back(Ok(StreamEvent::ContentBlockDelta {
                                    index: acc.3,
                                    delta: ContentDelta::InputJsonDelta {
                                        partial_json: args.clone(),
                                    },
                                }));
                            }
                        }
                    }
                }
            }

            // Handle finish_reason.
            if let Some(ref reason) = choice.finish_reason {
                // Close any open text block.
                if let Some(text_idx) = self.text_block_index.take() {
                    self.ready
                        .push_back(Ok(StreamEvent::ContentBlockStop { index: text_idx }));
                }
                // Close any open tool call blocks.
                let tool_block_indices: Vec<u32> = self
                    .tool_accumulators
                    .values()
                    .map(|acc| acc.3)
                    .collect();
                for idx in tool_block_indices {
                    self.ready
                        .push_back(Ok(StreamEvent::ContentBlockStop { index: idx }));
                }

                let stop_reason = match reason.as_str() {
                    "tool_calls" => StopReason::ToolUse,
                    "length" => StopReason::MaxTokens,
                    "stop" => StopReason::EndTurn,
                    _ => StopReason::EndTurn,
                };
                self.ready.push_back(Ok(StreamEvent::MessageDelta {
                    stop_reason,
                    usage: TokenUsage::default(),
                }));
            }
        }
    }

    fn emit_stream_end(&mut self) {
        self.ready.push_back(Ok(StreamEvent::MessageStop));
    }

    fn next_event(&mut self) -> Option<Result<StreamEvent>> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Option<Result<StreamEvent>> {
        // Process any remaining bytes.
        if !self.raw_tail.is_empty() {
            let leftover = String::from_utf8_lossy(&self.raw_tail).to_string();
            self.buffer.push_str(&leftover);
            self.raw_tail.clear();
        }
        self.parse_lines();

        if let Some(ev) = self.ready.pop_front() {
            return Some(ev);
        }

        // If we never got [DONE] but the stream ended, emit stop.
        if self.started {
            self.started = false; // prevent double-emit
            return Some(Ok(StreamEvent::MessageStop));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_normalization_openai() {
        let p = OpenAiProvider::new("key", "gpt-4", "https://api.openai.com/v1/");
        assert_eq!(p.completions_url(), "https://api.openai.com/v1/chat/completions");

        let p = OpenAiProvider::new("key", "gpt-4", "https://api.openai.com/v1");
        assert_eq!(p.completions_url(), "https://api.openai.com/v1/chat/completions");

        let p = OpenAiProvider::new("key", "gpt-4", "https://api.openai.com");
        assert_eq!(p.completions_url(), "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn url_normalization_zhipu() {
        // ZhiPu uses /api/paas/v4/ — should NOT prepend /v1.
        let p = OpenAiProvider::new("key", "glm-5.1", "https://open.bigmodel.cn/api/paas/v4/");
        assert_eq!(
            p.completions_url(),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
    }

    #[test]
    fn url_normalization_deepseek() {
        let p = OpenAiProvider::new("key", "deepseek-chat", "https://api.deepseek.com");
        assert_eq!(p.completions_url(), "https://api.deepseek.com/v1/chat/completions");
    }

    #[test]
    fn build_request_basic_text() {
        let p = OpenAiProvider::new("key", "gpt-4", "https://api.openai.com");
        let req = MessageRequest {
            model: "gpt-4".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: MessageContent::Text("Hello".to_string()),
            }],
            tools: None,
            max_tokens: 1024,
            stream: false,
            system: Some("You are helpful.".to_string()),
        };
        let api_req = p.build_request(&req);
        assert_eq!(api_req.messages.len(), 2); // system + user
        assert_eq!(api_req.messages[0].role, "system");
        assert_eq!(api_req.messages[1].role, "user");
        assert_eq!(api_req.messages[1].content.as_deref(), Some("Hello"));
    }

    #[test]
    fn build_request_with_tool_calls() {
        let p = OpenAiProvider::new("key", "gpt-4", "https://api.openai.com");
        let req = MessageRequest {
            model: "gpt-4".to_string(),
            messages: vec![
                Message {
                    role: "user".to_string(),
                    content: MessageContent::Text("Read the file".to_string()),
                },
                Message {
                    role: "assistant".to_string(),
                    content: MessageContent::Blocks(vec![
                        ContentBlock::Text {
                            text: "I'll read it.".to_string(),
                        },
                        ContentBlock::ToolUse {
                            id: "call_1".to_string(),
                            name: "read_file".to_string(),
                            input: serde_json::json!({"path": "src/main.rs"}),
                        },
                    ]),
                },
                Message {
                    role: "user".to_string(),
                    content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: "call_1".to_string(),
                        content: "fn main() {}".to_string(),
                        is_error: false,
                    }]),
                },
            ],
            tools: Some(vec![ToolDefinition {
                name: "read_file".to_string(),
                description: Some("Read a file".to_string()),
                input_schema: Some(serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}})),
            }]),
            max_tokens: 1024,
            stream: false,
            system: None,
        };
        let api_req = p.build_request(&req);
        // user, assistant (with tool_calls), tool
        assert_eq!(api_req.messages.len(), 3);
        assert_eq!(api_req.messages[1].role, "assistant");
        assert!(api_req.messages[1].tool_calls.is_some());
        assert_eq!(api_req.messages[2].role, "tool");
        assert_eq!(
            api_req.messages[2].tool_call_id.as_deref(),
            Some("call_1")
        );
        assert!(api_req.tools.is_some());
        assert_eq!(api_req.tools.as_ref().unwrap()[0].function.name, "read_file");
    }

    #[test]
    fn parse_non_streaming_response() {
        let resp: OpenAiResponse = serde_json::from_str(
            r#"{
                "id": "chatcmpl-123",
                "model": "gpt-4",
                "choices": [{
                    "message": {
                        "content": "Hello there!",
                        "tool_calls": null
                    },
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5}
            }"#,
        )
        .unwrap();
        let parsed = parse_response(resp);
        assert_eq!(parsed.id, "chatcmpl-123");
        assert_eq!(parsed.content.len(), 1);
        assert!(matches!(&parsed.content[0], ContentBlock::Text { text } if text == "Hello there!"));
        assert_eq!(parsed.stop_reason, StopReason::EndTurn);
        assert_eq!(parsed.usage.input_tokens, 10);
    }

    #[test]
    fn parse_non_streaming_tool_call() {
        let resp: OpenAiResponse = serde_json::from_str(
            r#"{
                "id": "chatcmpl-456",
                "model": "gpt-4",
                "choices": [{
                    "message": {
                        "content": null,
                        "tool_calls": [{
                            "id": "call_abc",
                            "type": "function",
                            "function": {
                                "name": "read_file",
                                "arguments": "{\"path\":\"src/main.rs\"}"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {"prompt_tokens": 20, "completion_tokens": 10}
            }"#,
        )
        .unwrap();
        let parsed = parse_response(resp);
        assert_eq!(parsed.stop_reason, StopReason::ToolUse);
        assert_eq!(parsed.content.len(), 1);
        match &parsed.content[0] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "call_abc");
                assert_eq!(name, "read_file");
                assert_eq!(input["path"], "src/main.rs");
            }
            other => panic!("Expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn sse_parser_text_stream() {
        let mut parser = OpenAiSseParser::new();
        let sse = concat!(
            "data: {\"id\":\"c1\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}],\"usage\":null}\n\n",
            "data: {\"id\":\"c1\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}],\"usage\":null}\n\n",
            "data: {\"id\":\"c1\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":null}],\"usage\":null}\n\n",
            "data: {\"id\":\"c1\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":null}\n\n",
            "data: [DONE]\n\n",
        );
        parser.feed(sse.as_bytes());

        // MessageStart
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::MessageStart { .. }));

        // ContentBlockStart (text)
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::ContentBlockStart { .. }));

        // TextDelta "Hello"
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::ContentBlockDelta { delta: ContentDelta::TextDelta { ref text }, .. } if text == "Hello"));

        // TextDelta " world"
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::ContentBlockDelta { delta: ContentDelta::TextDelta { ref text }, .. } if text == " world"));

        // ContentBlockStop (text closed by finish_reason)
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::ContentBlockStop { .. }));

        // MessageDelta (stop)
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::MessageDelta { stop_reason: StopReason::EndTurn, .. }));

        // MessageStop
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::MessageStop));
    }

    #[test]
    fn sse_parser_tool_call_stream() {
        let mut parser = OpenAiSseParser::new();
        let sse = concat!(
            "data: {\"id\":\"c2\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"read_file\",\"arguments\":\"\"}}]},\"finish_reason\":null}],\"usage\":null}\n\n",
            "data: {\"id\":\"c2\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\"\"}}]},\"finish_reason\":null}],\"usage\":null}\n\n",
            "data: {\"id\":\"c2\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\": \\\"src/main.rs\\\"}\"}}]},\"finish_reason\":null}],\"usage\":null}\n\n",
            "data: {\"id\":\"c2\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":null}\n\n",
            "data: [DONE]\n\n",
        );
        parser.feed(sse.as_bytes());

        // MessageStart
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::MessageStart { .. }));

        // ContentBlockStart (tool_use)
        let ev = parser.next_event().unwrap().unwrap();
        match ev {
            StreamEvent::ContentBlockStart {
                content_block: ContentBlock::ToolUse { ref name, .. },
                ..
            } => assert_eq!(name, "read_file"),
            other => panic!("Expected ContentBlockStart/ToolUse, got {other:?}"),
        }

        // InputJsonDelta fragments
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::ContentBlockDelta { delta: ContentDelta::InputJsonDelta { .. }, .. }));

        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::ContentBlockDelta { delta: ContentDelta::InputJsonDelta { .. }, .. }));

        // ContentBlockStop (from finish_reason)
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::ContentBlockStop { .. }));

        // MessageDelta (tool_calls)
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::MessageDelta { stop_reason: StopReason::ToolUse, .. }));

        // MessageStop
        let ev = parser.next_event().unwrap().unwrap();
        assert!(matches!(ev, StreamEvent::MessageStop));
    }
}
