//! Execution policy engine.
//!
//! Evaluates every tool action through a typed policy decision instead of
//! ad hoc prompt rules. Separates policy decisions from execution so a
//! future sandbox transform layer can operate independently.
//!
//! Ported from `lib/exec-policy.js` — this is the Rust-native owner.

mod classify;
mod evaluate;
mod paths;

pub use classify::{classify_tool_action, Classification};
pub use evaluate::{evaluate_policy, record_approval, PermissionAmendments, PolicyResult};
pub use paths::{is_in_workspace, is_protected_path};

use serde::{Deserialize, Serialize};
use std::fmt;

// ── Tool classes ────────────────────────────────────────────────────────────

/// Risk-based classification of a tool invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolClass {
    Read,
    Write,
    Exec,
    Destructive,
}

impl fmt::Display for ToolClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => write!(f, "read"),
            Self::Write => write!(f, "write"),
            Self::Exec => write!(f, "exec"),
            Self::Destructive => write!(f, "destructive"),
        }
    }
}

// ── Policy modes ────────────────────────────────────────────────────────────

/// How strictly the engine gates non-read operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyMode {
    /// read=auto, write/exec=ask-once-per-class, destructive=always-ask
    #[serde(rename = "auto")]
    Auto,
    /// read=auto, everything else denied
    #[serde(rename = "read-only")]
    ReadOnly,
    /// read=auto, all other classes always-ask
    #[serde(rename = "granular")]
    Granular,
    /// everything auto-approved (--trust)
    #[serde(rename = "full-access")]
    FullAccess,
}

impl PolicyMode {
    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "auto" => Self::Auto,
            "read-only" => Self::ReadOnly,
            "granular" => Self::Granular,
            "full-access" => Self::FullAccess,
            _ => Self::Auto,
        }
    }
}

// ── Decisions ───────────────────────────────────────────────────────────────

/// The three possible outcomes of a policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Proceed without user interaction.
    Allow,
    /// Block the operation — do not execute.
    Deny,
    /// Pause and ask the user for confirmation.
    Ask,
}
