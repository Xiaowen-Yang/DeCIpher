//! Policy evaluation — the core decision engine.
//!
//! Ported from `evaluatePolicy` in `lib/exec-policy.js`.

use std::collections::HashSet;

use serde_json::Value;

use crate::classify::classify_tool_action;
use crate::paths::{is_in_workspace, is_protected_path, resolve_path};
use crate::{Decision, PolicyMode, ToolClass};

// ── Permission amendments ───────────────────────────────────────────────────

/// Per-session approval tracker.
///
/// Unlike a session-wide boolean, approving `write` does not imply
/// approval for `destructive` operations.
#[derive(Debug, Clone)]
pub struct PermissionAmendments {
    pub approved_classes: HashSet<ToolClass>,
    pub approved_tools: HashSet<String>,
    pub path_carveouts: Vec<String>,
}

impl PermissionAmendments {
    pub fn new() -> Self {
        Self {
            approved_classes: HashSet::new(),
            approved_tools: HashSet::new(),
            path_carveouts: Vec::new(),
        }
    }
}

impl Default for PermissionAmendments {
    fn default() -> Self {
        Self::new()
    }
}

// ── Policy result ───────────────────────────────────────────────────────────

/// The outcome of evaluating a tool action against the current policy.
#[derive(Debug, Clone)]
pub struct PolicyResult {
    pub decision: Decision,
    pub tool_class: ToolClass,
    pub reason: String,
    pub protected_path: Option<String>,
}

// ── Evaluation ──────────────────────────────────────────────────────────────

/// Evaluate a tool action against the current policy.
///
/// This is the canonical entry point — mirrors `evaluatePolicy()` in
/// `lib/exec-policy.js`.
pub fn evaluate_policy(
    mode: PolicyMode,
    tool_name: &str,
    args: &Value,
    amendments: &PermissionAmendments,
    workspace: Option<&str>,
) -> PolicyResult {
    let classification = classify_tool_action(tool_name, args);
    let tool_class = classification.tool_class;
    let reason = classification.reason.clone();

    // Normalize all paths relative to workspace.
    // Prevents "../outside" from bypassing workspace boundaries.
    let paths: Vec<String> = classification
        .paths
        .iter()
        .map(|p| resolve_path(p, workspace))
        .collect();

    // Build effective carveouts: workspace + /tmp + user-defined
    let mut carveout_strs: Vec<String> = amendments.path_carveouts.clone();
    if let Some(ws) = workspace {
        carveout_strs.push(resolve_path(ws, None));
    }
    carveout_strs.push("/tmp".to_string());
    let carveouts: Vec<&str> = carveout_strs.iter().map(|s| s.as_str()).collect();

    // Check protected paths for non-read operations
    if tool_class != ToolClass::Read {
        for p in &paths {
            if let Some(pattern) = is_protected_path(p, &carveouts) {
                return PolicyResult {
                    decision: Decision::Deny,
                    tool_class,
                    reason: format!("protected path: {p} ({pattern})"),
                    protected_path: Some(p.clone()),
                };
            }
        }

        // Enforce workspace boundary
        if let Some(ws) = workspace {
            for p in &paths {
                if !is_in_workspace(p, ws) {
                    return PolicyResult {
                        decision: Decision::Deny,
                        tool_class,
                        reason: format!("outside workspace: {p}"),
                        protected_path: Some(p.clone()),
                    };
                }
            }
        }
    }

    // Policy mode evaluation
    match mode {
        PolicyMode::FullAccess => PolicyResult {
            decision: Decision::Allow,
            tool_class,
            reason,
            protected_path: None,
        },

        PolicyMode::ReadOnly => {
            if tool_class == ToolClass::Read {
                PolicyResult {
                    decision: Decision::Allow,
                    tool_class,
                    reason,
                    protected_path: None,
                }
            } else {
                PolicyResult {
                    decision: Decision::Deny,
                    tool_class,
                    reason: format!("read-only mode: {reason}"),
                    protected_path: None,
                }
            }
        }

        PolicyMode::Granular => {
            if tool_class == ToolClass::Read {
                PolicyResult {
                    decision: Decision::Allow,
                    tool_class,
                    reason,
                    protected_path: None,
                }
            } else {
                PolicyResult {
                    decision: Decision::Ask,
                    tool_class,
                    reason,
                    protected_path: None,
                }
            }
        }

        PolicyMode::Auto => {
            // Read: always auto-approved
            if tool_class == ToolClass::Read {
                return PolicyResult {
                    decision: Decision::Allow,
                    tool_class,
                    reason,
                    protected_path: None,
                };
            }

            // Destructive: always requires confirmation
            if tool_class == ToolClass::Destructive {
                return PolicyResult {
                    decision: Decision::Ask,
                    tool_class,
                    reason,
                    protected_path: None,
                };
            }

            // Write and exec: ask once per class, then auto-approve
            if amendments.approved_classes.contains(&tool_class) {
                return PolicyResult {
                    decision: Decision::Allow,
                    tool_class,
                    reason,
                    protected_path: None,
                };
            }
            if amendments.approved_tools.contains(tool_name) {
                return PolicyResult {
                    decision: Decision::Allow,
                    tool_class,
                    reason,
                    protected_path: None,
                };
            }

            PolicyResult {
                decision: Decision::Ask,
                tool_class,
                reason,
                protected_path: None,
            }
        }
    }
}

/// Record an approval for a tool class (ask-once-per-class pattern).
pub fn record_approval(amendments: &mut PermissionAmendments, tool_class: ToolClass, tool_name: Option<&str>) {
    amendments.approved_classes.insert(tool_class);
    if let Some(name) = tool_name {
        amendments.approved_tools.insert(name.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_amendments() -> PermissionAmendments {
        PermissionAmendments::new()
    }

    // ── Auto mode ───────────────────────────────────────────���───────────

    #[test]
    fn auto_read_always_allowed() {
        let r = evaluate_policy(
            PolicyMode::Auto, "read_file", &json!({"path": "src/main.rs"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Allow);
        assert_eq!(r.tool_class, ToolClass::Read);
    }

    #[test]
    fn auto_write_asks_first_time() {
        let r = evaluate_policy(
            PolicyMode::Auto, "write_file", &json!({"path": "src/out.txt"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Ask);
        assert_eq!(r.tool_class, ToolClass::Write);
    }

    #[test]
    fn auto_write_allowed_after_approval() {
        let mut amend = empty_amendments();
        record_approval(&mut amend, ToolClass::Write, Some("write_file"));

        let r = evaluate_policy(
            PolicyMode::Auto, "write_file", &json!({"path": "src/out.txt"}),
            &amend, Some("/app"),
        );
        assert_eq!(r.decision, Decision::Allow);
    }

    #[test]
    fn auto_exec_asks_first_time() {
        let r = evaluate_policy(
            PolicyMode::Auto, "exec_command", &json!({"cmd": "npm test"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Ask);
        assert_eq!(r.tool_class, ToolClass::Exec);
    }

    #[test]
    fn auto_exec_allowed_after_class_approval() {
        let mut amend = empty_amendments();
        record_approval(&mut amend, ToolClass::Exec, None);

        let r = evaluate_policy(
            PolicyMode::Auto, "exec_command", &json!({"cmd": "cargo build"}),
            &amend, Some("/app"),
        );
        assert_eq!(r.decision, Decision::Allow);
    }

    #[test]
    fn auto_destructive_always_asks() {
        let mut amend = empty_amendments();
        // Even with exec approved, destructive still asks
        record_approval(&mut amend, ToolClass::Exec, None);

        let r = evaluate_policy(
            PolicyMode::Auto, "exec_command", &json!({"cmd": "rm -rf /tmp/build"}),
            &amend, Some("/app"),
        );
        assert_eq!(r.decision, Decision::Ask);
        assert_eq!(r.tool_class, ToolClass::Destructive);
    }

    #[test]
    fn auto_write_approval_does_not_cover_exec() {
        let mut amend = empty_amendments();
        record_approval(&mut amend, ToolClass::Write, None);

        let r = evaluate_policy(
            PolicyMode::Auto, "exec_command", &json!({"cmd": "npm test"}),
            &amend, Some("/app"),
        );
        assert_eq!(r.decision, Decision::Ask);
    }

    // ── Read-only mode ──────────────────────────────────────────────────

    #[test]
    fn read_only_allows_read() {
        let r = evaluate_policy(
            PolicyMode::ReadOnly, "read_file", &json!({"path": "main.rs"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Allow);
    }

    #[test]
    fn read_only_denies_write() {
        let r = evaluate_policy(
            PolicyMode::ReadOnly, "write_file", &json!({"path": "out.txt"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Deny);
    }

    #[test]
    fn read_only_denies_exec() {
        let r = evaluate_policy(
            PolicyMode::ReadOnly, "exec_command", &json!({"cmd": "npm test"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Deny);
    }

    // ── Granular mode ───────────────────────────────────────────────────

    #[test]
    fn granular_allows_read() {
        let r = evaluate_policy(
            PolicyMode::Granular, "read_file", &json!({"path": "x.rs"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Allow);
    }

    #[test]
    fn granular_always_asks_for_write() {
        let r = evaluate_policy(
            PolicyMode::Granular, "write_file", &json!({"path": "out.txt"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Ask);
    }

    #[test]
    fn granular_always_asks_for_exec() {
        let r = evaluate_policy(
            PolicyMode::Granular, "exec_command", &json!({"cmd": "npm test"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Ask);
    }

    // ── Full-access mode ────────────────────────────────────────────────

    #[test]
    fn full_access_allows_everything() {
        let r = evaluate_policy(
            PolicyMode::FullAccess, "exec_command", &json!({"cmd": "npm test"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Allow);
    }

    #[test]
    fn full_access_allows_destructive() {
        let r = evaluate_policy(
            PolicyMode::FullAccess, "exec_command", &json!({"cmd": "rm -rf /app/build"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Allow);
    }

    // ── Protected paths ─────────────────────────────────────────────────

    #[test]
    fn tool_call_denied_protected_path() {
        let r = evaluate_policy(
            PolicyMode::FullAccess, "write_file", &json!({"path": "/home/user/.ssh/id_rsa"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Deny);
        assert!(r.protected_path.is_some());
        assert!(r.reason.contains("protected path"));
    }

    #[test]
    fn protected_path_denied_git_dir() {
        let r = evaluate_policy(
            PolicyMode::Auto, "write_file", &json!({"path": "/app/.git/config"}),
            &empty_amendments(), Some("/not-app"),
        );
        assert_eq!(r.decision, Decision::Deny);
        assert!(r.reason.contains(".git"));
    }

    #[test]
    fn protected_path_denied_env_file() {
        let r = evaluate_policy(
            PolicyMode::FullAccess, "write_file", &json!({"path": "/other/.env"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Deny);
    }

    #[test]
    fn protected_path_carveout_allows() {
        let mut amend = empty_amendments();
        amend.path_carveouts.push("/app/.git".to_string());

        let r = evaluate_policy(
            PolicyMode::FullAccess, "write_file", &json!({"path": "/app/.git/config"}),
            &amend, Some("/app"),
        );
        // Workspace is /app, path starts with workspace → not outside.
        // Carveout covers /app/.git → not protected.
        assert_eq!(r.decision, Decision::Allow);
    }

    // ── Workspace boundary ──────────────────────────────────────────────

    #[test]
    fn write_outside_workspace_denied() {
        let r = evaluate_policy(
            PolicyMode::FullAccess, "write_file", &json!({"path": "/etc/hosts"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Deny);
        assert!(r.reason.contains("outside workspace") || r.reason.contains("protected path"));
    }

    #[test]
    fn write_to_tmp_allowed() {
        let r = evaluate_policy(
            PolicyMode::FullAccess, "write_file", &json!({"path": "/tmp/output.txt"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Allow);
    }

    #[test]
    fn read_outside_workspace_allowed() {
        // Reads are not restricted by workspace boundary
        let r = evaluate_policy(
            PolicyMode::Auto, "read_file", &json!({"path": "/etc/hosts"}),
            &empty_amendments(), Some("/app"),
        );
        assert_eq!(r.decision, Decision::Allow);
    }

    // ── Approval prompt flow ────────────────────────────────────────────

    #[test]
    fn approval_prompt_flow() {
        let mut amend = empty_amendments();

        // First write → Ask
        let r1 = evaluate_policy(
            PolicyMode::Auto, "write_file", &json!({"path": "out.txt"}),
            &amend, Some("/app"),
        );
        assert_eq!(r1.decision, Decision::Ask);

        // User approves → record
        record_approval(&mut amend, r1.tool_class, Some("write_file"));

        // Second write → Allow (class approved)
        let r2 = evaluate_policy(
            PolicyMode::Auto, "write_file", &json!({"path": "other.txt"}),
            &amend, Some("/app"),
        );
        assert_eq!(r2.decision, Decision::Allow);

        // Exec still asks (different class)
        let r3 = evaluate_policy(
            PolicyMode::Auto, "exec_command", &json!({"cmd": "npm test"}),
            &amend, Some("/app"),
        );
        assert_eq!(r3.decision, Decision::Ask);
    }

    // ── PolicyMode parsing ──────────────────────────────────────────────

    #[test]
    fn policy_mode_from_str() {
        assert_eq!(PolicyMode::from_str_loose("auto"), PolicyMode::Auto);
        assert_eq!(PolicyMode::from_str_loose("read-only"), PolicyMode::ReadOnly);
        assert_eq!(PolicyMode::from_str_loose("granular"), PolicyMode::Granular);
        assert_eq!(PolicyMode::from_str_loose("full-access"), PolicyMode::FullAccess);
        assert_eq!(PolicyMode::from_str_loose("unknown"), PolicyMode::Auto);
    }

    // ── Relative path traversal ─────────────────────────────────────────

    #[test]
    fn dotdot_traversal_denied() {
        let r = evaluate_policy(
            PolicyMode::FullAccess, "write_file", &json!({"path": "../../../etc/passwd"}),
            &empty_amendments(), Some("/app/project"),
        );
        assert_eq!(r.decision, Decision::Deny);
    }
}
