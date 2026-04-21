//! Tool execution for the Rust-native agent loop.
//!
//! Each tool handler takes structured args (as `serde_json::Value`) and a
//! `ToolContext` carrying the working directory and an output streaming
//! callback.  Handlers return a `ToolOutput` that is formatted and fed back
//! into the conversation history.
//!
//! Port source: `agents/executor/tools.js`

pub mod exec;
pub mod files;
pub mod patch;
pub mod search;
pub mod subagent;

use std::sync::Arc;

use serde_json::Value;

/// Shared context threaded through all tool handlers.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Absolute path to the working directory.
    pub workspace: String,
    /// Optional callback receiving live output chunks from exec_command.
    pub on_exec_output: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    /// MCP clients — shared, each protected by a tokio Mutex.
    pub mcp_clients: Option<Arc<Vec<Arc<tokio::sync::Mutex<decipher_mcp::McpClient>>>>>,
    /// API key — needed for spawning subagents.
    pub api_key: String,
    /// Model name — needed for spawning subagents.
    pub model: String,
    /// Base URL override for API calls.
    pub base_url: Option<String>,
    /// Event channel — for forwarding subagent events to parent TUI.
    pub event_tx: Option<tokio::sync::mpsc::Sender<decipher_protocol::ServerMessage>>,
    /// Nesting depth: 0 = top-level agent, 1 = subagent, etc.
    pub depth: u8,
    /// Policy mode inherited from the parent agent config.
    pub policy_mode: decipher_policy::PolicyMode,
}

/// Result from a single tool invocation.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub success: bool,
    /// Human-readable summary for the TUI.
    pub summary: String,
    /// Full text to return to the LLM in the tool_result message.
    pub llm_text: String,
    /// Process exit code (exec_command only).
    pub exit_code: Option<i32>,
    /// Raw output for streaming preview (exec_command / search).
    pub raw_output: Option<String>,
    /// Structured parsed output for TUI smart-card rendering (exec_command only).
    /// Never sent to the LLM — display layer only.
    pub parsed_output: Option<crate::output_parser::ParsedOutput>,
}

impl ToolOutput {
    pub fn ok(summary: impl Into<String>, llm_text: impl Into<String>) -> Self {
        Self {
            success: true,
            summary: summary.into(),
            llm_text: llm_text.into(),
            exit_code: None,
            raw_output: None,
            parsed_output: None,
        }
    }

    pub fn err(summary: impl Into<String>, llm_text: impl Into<String>) -> Self {
        Self {
            success: false,
            summary: summary.into(),
            llm_text: llm_text.into(),
            exit_code: None,
            raw_output: None,
            parsed_output: None,
        }
    }
}

/// Dispatch a tool call by name.
///
/// Returns `Ok(ToolOutput)` even for tool-level errors (e.g. file not found).
/// The `Err` variant is reserved for panics / internal runtime failures.
pub async fn dispatch(
    name: &str,
    args: &Value,
    ctx: &ToolContext,
) -> Result<ToolOutput, crate::RuntimeError> {
    match name {
        "exec_command" => exec::run(args, ctx).await,
        "read_file" => files::read(args, ctx).await,
        "write_file" => files::write(args, ctx).await,
        "list_files" => files::list(args, ctx).await,
        "apply_patch" => patch::apply(args, ctx).await,
        "search" => search::search(args, ctx).await,
        "grep_search" => search::grep(args, ctx).await,
        "file_search" => search::file_search(args, ctx).await,
        "kubectl_get" => exec::kubectl(args, ctx, "get").await,
        "kubectl_logs" => exec::kubectl_logs(args, ctx).await,
        "kubectl_describe" => exec::kubectl(args, ctx, "describe").await,
        "kubectl_events" => exec::kubectl_events(args, ctx).await,
        "update_plan" => Ok(ToolOutput::ok(
            "Plan updated",
            format!("[Tool result: update_plan]\nPlan updated ({} steps).", {
                args.get("steps")
                    .and_then(Value::as_array)
                    .map(|a| a.len())
                    .unwrap_or(0)
            }),
        )),
        "done" => {
            // done is handled specially in the agent loop before dispatch.
            // If it reaches here something is wrong — return a stub.
            Ok(ToolOutput::ok("Done", "[Tool result: done]\nMission complete."))
        }
        "spawn_agent" => subagent::spawn_agent(args, ctx).await,
        unknown => {
            // Route prefixed MCP tool names: mcp__<server>_<tool>
            if let Some(ref mcp_clients) = ctx.mcp_clients {
                if let Some(rest) = unknown.strip_prefix("mcp__") {
                    for client_arc in mcp_clients.as_ref() {
                        let client_guard = client_arc.lock().await;
                        let server_prefix = format!("{}_", client_guard.server_name);
                        if let Some(tool_name) = rest.strip_prefix(&server_prefix) {
                            let tool_name = tool_name.to_string();
                            let server_name = client_guard.server_name.clone();
                            drop(client_guard);
                            let mut client = client_arc.lock().await;
                            match client.call_tool(&tool_name, args.clone()).await {
                                Ok(result) => {
                                    return Ok(ToolOutput::ok(
                                        format!("mcp:{server_name}"),
                                        format!("[Tool result: {tool_name}]\n{result}"),
                                    ));
                                }
                                Err(e) => {
                                    return Ok(ToolOutput::err(
                                        format!("MCP error: {e}"),
                                        format!("Error calling MCP tool \"{tool_name}\": {e}"),
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            Ok(ToolOutput::err(
                format!("Unknown tool: {unknown}"),
                format!(
                    "Error: Unknown tool \"{unknown}\". Available: exec_command, read_file, write_file, apply_patch, list_files, search, grep_search, file_search, done"
                ),
            ))
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Resolve a path that may be absolute or relative to the workspace.
pub(crate) fn resolve_path(workspace: &str, path: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::path::Path::new(workspace).join(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_absolute_path_unchanged() {
        let p = resolve_path("/workspace", "/etc/hosts");
        assert_eq!(p, std::path::PathBuf::from("/etc/hosts"));
    }

    #[test]
    fn resolve_relative_joins_workspace() {
        let p = resolve_path("/workspace", "src/main.rs");
        assert_eq!(p, std::path::PathBuf::from("/workspace/src/main.rs"));
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_error_output() {
        let ctx = ToolContext {
            workspace: "/tmp".to_string(),
            on_exec_output: None,
            mcp_clients: None,
            api_key: String::new(),
            model: String::new(),
            base_url: None,
            event_tx: None,
            depth: 0,
            policy_mode: decipher_policy::PolicyMode::Auto,
        };
        let out = dispatch("nonexistent_tool", &serde_json::json!({}), &ctx)
            .await
            .unwrap();
        assert!(!out.success);
        assert!(out.llm_text.contains("Unknown tool"));
    }
}
