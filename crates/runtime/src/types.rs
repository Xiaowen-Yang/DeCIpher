//! Public types for the runtime crate.

use serde::{Deserialize, Serialize};

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
            max_turns: 20,
            policy_mode: decipher_policy::PolicyMode::Auto,
            max_tokens: 8192,
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
