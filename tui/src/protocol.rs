#![allow(dead_code)]
//! JSON protocol types for TUI <-> Node.js agent communication.
//!
//! The Rust TUI spawns `node bin/decipher --server` and communicates via
//! newline-delimited JSON on stdin/stdout.

use serde::{Deserialize, Serialize};

// ── Messages FROM TUI TO agent ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "user_input")]
    UserInput {
        text: String,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        images: Vec<ImageData>,
    },

    #[serde(rename = "slash_command")]
    SlashCommand { name: String, args: Option<String> },

    #[serde(rename = "approval_response")]
    ApprovalResponse { approved: bool },

    #[serde(rename = "interrupt")]
    Interrupt,
}

#[derive(Debug, Serialize)]
pub struct ImageData {
    pub data: String, // base64-encoded PNG
    pub mime: String,
}

// ── Messages FROM agent TO TUI ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "banner")]
    Banner {
        version: String,
        provider: String,
        model: String,
        directory: String,
        api_key_set: bool,
    },

    #[serde(rename = "mission")]
    Mission {
        understood: String,
        target: Option<String>,
        target_type: Option<String>,
        steps: Vec<String>,
    },

    #[serde(rename = "clarification")]
    Clarification { question: String },

    #[serde(rename = "approval_request")]
    ApprovalRequest {
        capabilities: Vec<String>,
        action: Option<ActionDetail>,
    },

    #[serde(rename = "tool_start")]
    ToolStart { tool: String, reasoning: String },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool: String,
        success: bool,
        summary: String,
        elapsed_ms: u64,
    },

    #[serde(rename = "agent_message")]
    AgentMessage { text: String },

    #[serde(rename = "agent_message_delta")]
    AgentMessageDelta { delta: String },

    #[serde(rename = "mission_complete")]
    MissionComplete {
        outcome: String,
        summary: String,
        turns: u32,
        elapsed_ms: u64,
        urls: Vec<String>,
    },

    #[serde(rename = "error")]
    Error { message: String },

    #[serde(rename = "spinner")]
    Spinner { label: String, done: bool },

    #[serde(rename = "command_list")]
    CommandList { commands: Vec<CommandInfo> },
}

#[derive(Debug, Deserialize)]
pub struct ActionDetail {
    pub tool: String,
    pub reasoning: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CommandInfo {
    pub name: String,
    pub description: String,
}
