use serde::{Deserialize, Serialize};

// ── Request types (what the client sends) ──────────────────────────

#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<InputMessage>,
    #[serde(default)]
    pub stream: bool,
    pub system: Option<String>,
    pub tools: Option<Vec<ToolDefinition>>,
}

#[derive(Debug, Deserialize)]
pub struct InputMessage {
    pub role: String,
    pub content: InputContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum InputContent {
    Text(String),
    Blocks(Vec<InputContentBlock>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum InputContentBlock {
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
        content: ToolResultContent,
        #[serde(default)]
        is_error: bool,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(String),
    Blocks(Vec<ToolResultBlock>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ToolResultBlock {
    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Debug, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<serde_json::Value>,
}

// ── Response types (what the mock returns) ─────────────────────────

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub role: String,
    pub content: Vec<OutputContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub usage: Usage,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum OutputContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub cache_creation_input_tokens: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub cache_read_input_tokens: u32,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

impl Usage {
    pub fn new(input: u32, output: u32) -> Self {
        Self {
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }
    }
}

impl Default for Usage {
    fn default() -> Self {
        Self::new(100, 50)
    }
}

pub const DEFAULT_MODEL: &str = "claude-sonnet-4-5-20250514";

impl MessageResponse {
    pub fn text(id: &str, text: &str) -> Self {
        Self {
            id: id.to_string(),
            kind: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![OutputContentBlock::Text {
                text: text.to_string(),
            }],
            model: DEFAULT_MODEL.to_string(),
            stop_reason: Some("end_turn".to_string()),
            usage: Usage::default(),
        }
    }

    pub fn tool_use(id: &str, tool_id: &str, tool_name: &str, input: serde_json::Value) -> Self {
        Self {
            id: id.to_string(),
            kind: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![OutputContentBlock::ToolUse {
                id: tool_id.to_string(),
                name: tool_name.to_string(),
                input,
            }],
            model: DEFAULT_MODEL.to_string(),
            stop_reason: Some("tool_use".to_string()),
            usage: Usage::default(),
        }
    }

    pub fn multi_tool_use(id: &str, tools: Vec<(String, String, serde_json::Value)>) -> Self {
        let content = tools
            .into_iter()
            .map(|(tool_id, name, input)| OutputContentBlock::ToolUse {
                id: tool_id,
                name,
                input,
            })
            .collect();
        Self {
            id: id.to_string(),
            kind: "message".to_string(),
            role: "assistant".to_string(),
            content,
            model: DEFAULT_MODEL.to_string(),
            stop_reason: Some("tool_use".to_string()),
            usage: Usage::default(),
        }
    }
}

// ── Request inspection helpers ─────────────────────────────────────

impl MessageRequest {
    /// Extract the text from the latest tool_result in the conversation.
    pub fn latest_tool_result(&self) -> Option<String> {
        for msg in self.messages.iter().rev() {
            if msg.role != "user" {
                continue;
            }
            match &msg.content {
                InputContent::Blocks(blocks) => {
                    for block in blocks.iter().rev() {
                        if let InputContentBlock::ToolResult { content, .. } = block {
                            return Some(tool_result_text(content));
                        }
                    }
                }
                InputContent::Text(_) => {}
            }
        }
        None
    }

    /// Map tool names to their result text, for multi-tool responses.
    pub fn tool_results_by_id(&self) -> Vec<(String, String, bool)> {
        let mut results = Vec::new();
        for msg in &self.messages {
            if msg.role != "user" {
                continue;
            }
            if let InputContent::Blocks(blocks) = &msg.content {
                for block in blocks {
                    if let InputContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } = block
                    {
                        results.push((
                            tool_use_id.clone(),
                            tool_result_text(content),
                            *is_error,
                        ));
                    }
                }
            }
        }
        results
    }
}

fn tool_result_text(content: &ToolResultContent) -> String {
    match content {
        ToolResultContent::Text(t) => t.clone(),
        ToolResultContent::Blocks(blocks) => blocks
            .iter()
            .map(|b| match b {
                ToolResultBlock::Text { text } => text.as_str(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}
