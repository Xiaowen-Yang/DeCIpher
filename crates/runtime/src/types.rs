//! Public types for the runtime crate.

use decipher_providers::types::Message;
use serde::{Deserialize, Serialize};

use crate::git_context::GitContext;
use crate::hooks::HookConfig;
use crate::instructions::InstructionFiles;
use crate::skills::Skill;

/// Configuration for a single agent run.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// LLM model identifier (e.g. "claude-sonnet-4-5-20250514").
    pub model: String,
    /// Anthropic API key.
    pub api_key: String,
    /// Base URL for the API (default: Anthropic production endpoint).
    pub base_url: Option<String>,
    /// Working directory for file operations and process execution.
    pub workspace: String,
    /// Mission goal text fed to the agent system prompt.
    pub mission_goal: String,
    /// Optional pre-planned steps to include in the system prompt.
    pub plan_steps: Vec<String>,
    /// Maximum number of turns before aborting.
    pub max_turns: u32,
    /// Policy mode governing approval requirements.
    pub policy_mode: decipher_policy::PolicyMode,
    /// Maximum output tokens per LLM call.
    pub max_tokens: u32,
    /// Pre-loaded message history for session resume.
    /// When `Some`, the agent loop uses this as the initial history instead of
    /// building a fresh first user message from `mission_goal`.
    pub resume_from: Option<Vec<Message>>,
    /// Loaded skills to inject into the system prompt.
    pub skills: Vec<Skill>,
    /// Loaded instruction files (DECIPHER.md) for system prompt injection.
    pub instructions: InstructionFiles,
    /// Git context (branch, HEAD, dirty count) collected at session start.
    pub git_context: Option<GitContext>,
    /// Memory context string to inject into the system prompt.
    pub memory_context: Option<String>,
    /// If true, agent generates a plan without executing tools.
    pub plan_mode: bool,
    /// Hook configuration (shell hooks that fire around tool calls).
    pub hook_config: HookConfig,
    /// MCP tools discovered from connected servers (for injection into tool list).
    pub mcp_tools: Vec<decipher_mcp::McpTool>,
    /// Live MCP client connections (shared, one per server).
    pub mcp_clients: Option<std::sync::Arc<Vec<std::sync::Arc<tokio::sync::Mutex<decipher_mcp::McpClient>>>>>,
    /// Agent nesting depth: 0 = top-level, 1 = first subagent, etc.
    /// Used to enforce MAX_DEPTH in spawn_agent.
    pub depth: u8,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".to_string(),
            api_key: String::new(),
            base_url: None,
            workspace: std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            mission_goal: "Complete the requested task.".to_string(),
            plan_steps: Vec::new(),
            max_turns: 200,
            policy_mode: decipher_policy::PolicyMode::Auto,
            max_tokens: 8192,
            resume_from: None,
            skills: Vec::new(),
            instructions: InstructionFiles::default(),
            git_context: None,
            memory_context: None,
            plan_mode: false,
            hook_config: HookConfig::default(),
            mcp_tools: Vec::new(),
            mcp_clients: None,
            depth: 0,
        }
    }
}

/// The outcome reported by the `done` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunOutcome {
    Pass,
    Fail,
    Partial,
}

impl RunOutcome {
    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "PASS" => Self::Pass,
            "PARTIAL" => Self::Partial,
            _ => Self::Fail,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Partial => "PARTIAL",
        }
    }
}

/// Result of a completed agent run.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub outcome: RunOutcome,
    pub summary: String,
    pub turns_completed: u32,
    pub elapsed_ms: u64,
    pub files_modified: Vec<String>,
    pub errors_encountered: Vec<String>,
    pub next_steps: Vec<String>,
}

/// Errors from the runtime.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("Provider error: {0}")]
    Provider(#[from] decipher_providers::ProviderError),

    #[error("Tool execution error: {0}")]
    Tool(String),

    #[error("Max turns ({0}) exceeded without done call")]
    MaxTurnsExceeded(u32),

    #[error("Agent aborted: {0}")]
    Aborted(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Event channel closed")]
    ChannelClosed,
}
