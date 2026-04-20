#![allow(dead_code)]
//! Shared protocol types for DeCIpher TUI <-> agent communication.
//!
//! The Rust TUI spawns `node bin/decipher --server` and communicates via
//! newline-delimited JSON on stdin/stdout. This crate defines the message
//! types used by both sides.
//!
//! This is the **migration seam**: when the Node.js backend is replaced by
//! Rust, only the transport changes (subprocess → in-process channels).
//! The message types remain identical.

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

#[derive(Debug, Serialize, Deserialize)]
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
    ToolStart {
        tool: String,
        reasoning: String,
        /// Tool arguments (JSON object) — cmd for exec_command, path for read/write.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args: Option<serde_json::Value>,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool: String,
        success: bool,
        summary: String,
        elapsed_ms: u64,
        /// Process exit code (exec_command only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        /// First few lines of output for preview.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_preview: Option<String>,
        /// Total number of output lines.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_lines_total: Option<u32>,
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
        /// Files modified during the mission.
        #[serde(default)]
        files_modified: Vec<String>,
        /// Errors encountered during execution.
        #[serde(default)]
        errors_encountered: Vec<String>,
        /// Suggested next steps (for FAIL/PARTIAL outcomes).
        #[serde(default)]
        next_steps: Vec<String>,
    },

    #[serde(rename = "error")]
    Error { message: String },

    #[serde(rename = "spinner")]
    Spinner { label: String, done: bool },

    #[serde(rename = "command_list")]
    CommandList { commands: Vec<CommandInfo> },

    #[serde(rename = "token_usage")]
    TokenUsage {
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        /// Model context window size for budget bar display.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_window: Option<u64>,
    },

    /// Agent status update — phase, turn counter, elapsed time.
    #[serde(rename = "agent_status")]
    AgentStatus {
        phase: String,
        turn: u32,
        max_turns: u32,
        elapsed_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
    },

    /// Streaming output from a running exec_command (stdout/stderr chunks).
    #[serde(rename = "exec_output_delta")]
    ExecOutputDelta { delta: String },

    /// Native tool call from the LLM (parallel tool calling support).
    #[serde(rename = "tool_call")]
    ToolCall {
        call_id: String,
        name: String,
        /// JSON-encoded arguments.
        input: String,
    },

    /// Result of a native tool call execution.
    #[serde(rename = "tool_call_result")]
    ToolCallResult {
        call_id: String,
        name: String,
        output: String,
        success: bool,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActionDetail {
    pub tool: String,
    pub reasoning: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CommandInfo {
    pub name: String,
    pub description: String,
}
