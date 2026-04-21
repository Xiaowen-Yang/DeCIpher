//! Shell lifecycle hooks for DeCIpher.
//!
//! Hooks are shell scripts that fire at key agent lifecycle events.
//! Configuration is loaded from `~/.decipher/hooks.json`.
//!
//! # Hook types
//! - `PreToolUse`: fires before each tool call; can block execution
//! - `PostToolUse`: fires after each tool call
//! - `SessionStart`: fires when the agent loop starts
//! - `SessionEnd`: fires when the agent loop finishes
//!
//! # PreToolUse blocking
//! If any `PreToolUse` hook exits with code != 0 or outputs
//! `{"block": true, "reason": "..."}` on stdout, the tool call is blocked.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A single hook entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEntry {
    pub command: String,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Full hook configuration loaded from hooks.json.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookConfig {
    #[serde(rename = "PreToolUse", default)]
    pub pre_tool_use: Vec<HookEntry>,
    #[serde(rename = "PostToolUse", default)]
    pub post_tool_use: Vec<HookEntry>,
    #[serde(rename = "SessionStart", default)]
    pub session_start: Vec<HookEntry>,
    #[serde(rename = "SessionEnd", default)]
    pub session_end: Vec<HookEntry>,
}

impl HookConfig {
    /// Load hook config from `~/.decipher/hooks.json`.
    /// Returns an empty config if the file is missing or malformed.
    pub fn load(decipher_home: &Path) -> Self {
        let path = decipher_home.join("hooks.json");
        let Ok(data) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str::<HookConfig>(&data) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("[hooks] failed to parse {}: {e}", path.display());
                Self::default()
            }
        }
    }
}

/// Result of a PreToolUse hook check.
#[derive(Debug)]
pub struct PreHookResult {
    /// Whether the tool call should be blocked.
    pub blocked: bool,
    /// Reason for blocking (if blocked).
    pub reason: String,
}

/// Fire all PreToolUse hooks.
///
/// Returns `PreHookResult { blocked: false, .. }` if all hooks pass.
/// Returns `PreHookResult { blocked: true, reason }` if any hook blocks.
pub async fn fire_pre_tool_use(
    config: &HookConfig,
    tool: &str,
    args: &serde_json::Value,
    session_id: &str,
) -> PreHookResult {
    if config.pre_tool_use.is_empty() {
        return PreHookResult {
            blocked: false,
            reason: String::new(),
        };
    }

    let payload = serde_json::json!({
        "tool": tool,
        "args": args,
        "session_id": session_id,
    });
    let payload_str = payload.to_string();

    for hook in &config.pre_tool_use {
        let result = run_hook(hook, &payload_str).await;
        match result {
            HookRunResult::Blocked(reason) => {
                return PreHookResult {
                    blocked: true,
                    reason,
                };
            }
            HookRunResult::Failed(_) | HookRunResult::Ok(_) => {
                // Continue to next hook on non-blocking results.
            }
        }
    }

    PreHookResult {
        blocked: false,
        reason: String::new(),
    }
}

/// Fire all PostToolUse hooks (best-effort, errors are ignored).
pub async fn fire_post_tool_use(
    config: &HookConfig,
    tool: &str,
    success: bool,
    summary: &str,
    exit_code: Option<i32>,
) {
    if config.post_tool_use.is_empty() {
        return;
    }

    let payload = serde_json::json!({
        "tool": tool,
        "success": success,
        "summary": summary,
        "exit_code": exit_code,
    });
    let payload_str = payload.to_string();

    for hook in &config.post_tool_use {
        match run_hook(hook, &payload_str).await {
            HookRunResult::Failed(e) => eprintln!("[hooks] PostToolUse failed: {e}"),
            _ => {}
        }
    }
}

/// Fire a list of hooks (SessionStart / SessionEnd).
pub async fn fire_session_event(hooks: &[HookEntry]) {
    for hook in hooks {
        match run_hook(hook, "{}").await {
            HookRunResult::Failed(e) => eprintln!("[hooks] session hook failed: {e}"),
            _ => {}
        }
    }
}

// ── Internal helpers ───────────────────────────────────────────────────────────

enum HookRunResult {
    Ok(String),
    Blocked(String),
    Failed(String),
}

const HOOK_TIMEOUT: Duration = Duration::from_secs(30);

async fn run_hook(hook: &HookEntry, stdin_payload: &str) -> HookRunResult {
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;
    use tokio::time::timeout;

    // Parse command string into program + args (simple shell-word split).
    let parts = shell_words(&hook.command);
    if parts.is_empty() {
        return HookRunResult::Failed("empty command".into());
    }

    let mut cmd = Command::new(&parts[0]);
    cmd.args(&parts[1..]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());

    // Add extra env vars from hook config.
    for (k, v) in &hook.env {
        cmd.env(k, v);
    }

    let Ok(mut child) = cmd.spawn() else {
        return HookRunResult::Failed(format!("failed to spawn: {}", hook.command));
    };

    // Write payload to stdin then close the write end.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_payload.as_bytes()).await;
    }

    // Read stdout in a background task so the pipe buffer never fills
    // (avoids deadlock) and so we can still call child.kill() on timeout.
    let stdout_task = if let Some(out) = child.stdout.take() {
        use tokio::io::AsyncReadExt;
        Some(tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut r = out;
            let _ = r.read_to_end(&mut buf).await;
            buf
        }))
    } else {
        None
    };

    // Wait for exit with timeout; kill the process if it takes too long.
    let status = match timeout(HOOK_TIMEOUT, child.wait()).await {
        Ok(Ok(s)) => s,
        Ok(Err(_)) => return HookRunResult::Failed("[hooks] process error".into()),
        Err(_) => {
            let _ = child.kill().await;
            return HookRunResult::Failed(format!(
                "[hooks] timed out after {}s: {}",
                HOOK_TIMEOUT.as_secs(),
                hook.command
            ));
        }
    };

    let stdout_bytes = if let Some(task) = stdout_task {
        task.await.unwrap_or_default()
    } else {
        Vec::new()
    };

    // Re-assemble an Output-like view for the existing blocking/exit-code checks.
    let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
    let exit_success = status.success();

    // Check if hook requests blocking via JSON stdout.
    // Recognises both {"block": true} and {"action": "deny"} forms.
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&stdout) {
        let is_blocked = val.get("block").and_then(|b| b.as_bool()).unwrap_or(false)
            || val.get("action").and_then(|a| a.as_str()) == Some("deny");
        if is_blocked {
            let reason = val
                .get("reason")
                .and_then(|r| r.as_str())
                .unwrap_or("blocked by hook")
                .to_string();
            return HookRunResult::Blocked(reason);
        }
    }

    // Non-zero exit code also blocks for PreToolUse.
    if !exit_success {
        return HookRunResult::Blocked(format!(
            "hook exited with non-zero status: {}",
            hook.command
        ));
    }

    HookRunResult::Ok(stdout)
}

/// Very simple whitespace-splitting (no quote handling needed for basic hooks).
fn shell_words(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_returns_default_when_file_missing() {
        let cfg = HookConfig::load(Path::new("/nonexistent"));
        assert!(cfg.pre_tool_use.is_empty());
        assert!(cfg.post_tool_use.is_empty());
        assert!(cfg.session_start.is_empty());
        assert!(cfg.session_end.is_empty());
    }

    #[test]
    fn load_parses_hooks_json() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks_json = r#"{
            "PreToolUse": [{ "command": "/usr/bin/true" }],
            "PostToolUse": [{ "command": "/usr/bin/true", "env": { "FOO": "bar" } }],
            "SessionStart": [],
            "SessionEnd": [{ "command": "/usr/bin/echo done" }]
        }"#;
        fs::write(tmp.path().join("hooks.json"), hooks_json).unwrap();

        let cfg = HookConfig::load(tmp.path());
        assert_eq!(cfg.pre_tool_use.len(), 1);
        assert_eq!(cfg.post_tool_use.len(), 1);
        assert_eq!(cfg.post_tool_use[0].env.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(cfg.session_end.len(), 1);
    }

    #[tokio::test]
    async fn pre_tool_use_allows_when_no_hooks() {
        let cfg = HookConfig::default();
        let result = fire_pre_tool_use(&cfg, "read_file", &serde_json::json!({}), "session-1").await;
        assert!(!result.blocked);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn pre_tool_use_passes_with_true_command() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks_json = r#"{ "PreToolUse": [{ "command": "/usr/bin/true" }] }"#;
        fs::write(tmp.path().join("hooks.json"), hooks_json).unwrap();
        let cfg = HookConfig::load(tmp.path());
        let result = fire_pre_tool_use(&cfg, "exec_command", &serde_json::json!({}), "sess").await;
        assert!(!result.blocked);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn pre_tool_use_blocks_with_false_command() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks_json = r#"{ "PreToolUse": [{ "command": "/usr/bin/false" }] }"#;
        fs::write(tmp.path().join("hooks.json"), hooks_json).unwrap();
        let cfg = HookConfig::load(tmp.path());
        let result = fire_pre_tool_use(&cfg, "exec_command", &serde_json::json!({}), "sess").await;
        assert!(result.blocked);
    }

    #[tokio::test]
    async fn post_tool_use_no_hooks_is_noop() {
        let cfg = HookConfig::default();
        // Should complete without panicking.
        fire_post_tool_use(&cfg, "read_file", true, "ok", Some(0)).await;
    }

    #[tokio::test]
    async fn session_event_empty_hooks_is_noop() {
        fire_session_event(&[]).await;
    }

    #[test]
    fn shell_words_splits_correctly() {
        let parts = shell_words("/usr/bin/env FOO=bar");
        assert_eq!(parts, vec!["/usr/bin/env", "FOO=bar"]);
    }
}
