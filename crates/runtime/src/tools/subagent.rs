//! spawn_agent tool — spawns a sub-mission as a nested AgentLoop run.

use serde_json::Value;

use decipher_protocol::ServerMessage;
use decipher_providers::anthropic::AnthropicProvider;

use super::{ToolContext, ToolOutput};
use crate::agent_loop::AgentLoop;
use crate::types::AgentConfig;

/// Maximum subagent nesting depth.
const MAX_DEPTH: u8 = 2;

/// Execute a spawn_agent tool call.
pub async fn spawn_agent(
    args: &Value,
    ctx: &ToolContext,
) -> Result<ToolOutput, crate::RuntimeError> {
    // Guard: prevent runaway nesting.
    if ctx.depth >= MAX_DEPTH {
        return Ok(ToolOutput::err(
            format!("Subagent nesting limit ({MAX_DEPTH}) exceeded"),
            format!(
                "Error: spawn_agent nesting limit reached (depth {depth}). \
                 Cannot spawn a subagent from depth {depth}.",
                depth = ctx.depth
            ),
        ));
    }

    // Extract arguments.
    let task = match args.get("task").and_then(|v| v.as_str()) {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            return Ok(ToolOutput::err(
                "spawn_agent: missing task",
                "Error: spawn_agent requires a non-empty 'task' argument.",
            ));
        }
    };

    let workspace = args
        .get("workspace")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&ctx.workspace)
        .to_string();

    let max_turns = args
        .get("max_turns")
        .and_then(|v| v.as_u64())
        .map(|n| n.min(50) as u32)
        .unwrap_or(20);

    // Notify parent TUI that a subagent is starting.
    if let Some(ref tx) = ctx.event_tx {
        let _ = tx
            .send(ServerMessage::SubagentStart {
                task: task.clone(),
                depth: ctx.depth + 1,
            })
            .await;
    }

    // Build the subagent provider using same credentials.
    let sub_provider = {
        let mut p = AnthropicProvider::new(&ctx.api_key, &ctx.model);
        if let Some(ref url) = ctx.base_url {
            p = p.with_base_url(url);
        }
        p
    };

    // Create a channel to capture subagent events.
    let (sub_tx, mut sub_rx) = tokio::sync::mpsc::channel::<ServerMessage>(64);

    // Build subagent config — inherit credentials, set higher depth.
    let sub_cfg = AgentConfig {
        model: ctx.model.clone(),
        api_key: ctx.api_key.clone(),
        base_url: ctx.base_url.clone(),
        workspace: workspace.clone(),
        mission_goal: task.clone(),
        max_turns,
        // Subagent inherits depth+1 via ToolContext — not AgentConfig.
        // MCP clients are not inherited (avoid re-sharing handles across tasks).
        ..Default::default()
    };

    // Run the subagent inline (not in a separate tokio task) to avoid
    // the Send bound requirement that `AgentLoop::run` cannot satisfy
    // when MCP client handles are present in ToolContext.
    let parent_tx = ctx.event_tx.clone();
    let sub_depth = ctx.depth + 1;

    // Spawn event forwarding task — this only handles ServerMessage which is Send.
    let forward_handle = tokio::spawn(async move {
        let mut outcome = "FAIL".to_string();
        let mut summary = String::new();

        while let Some(msg) = sub_rx.recv().await {
            match &msg {
                ServerMessage::AgentMessage { text } => {
                    if let Some(ref tx) = parent_tx {
                        let _ = tx
                            .send(ServerMessage::AgentMessage {
                                text: format!("[↓ Sub@{sub_depth}] {text}"),
                            })
                            .await;
                    }
                }
                ServerMessage::ToolStart { tool, .. } => {
                    if let Some(ref tx) = parent_tx {
                        let _ = tx
                            .send(ServerMessage::AgentMessage {
                                text: format!("[↓ Sub@{sub_depth}] → {tool}"),
                            })
                            .await;
                    }
                }
                ServerMessage::MissionComplete {
                    outcome: o,
                    summary: s,
                    ..
                } => {
                    outcome = o.clone();
                    summary = s.clone();
                }
                _ => {}
            }
        }
        (outcome, summary)
    });

    // Run subagent directly (await, not spawn) to bypass Send requirement.
    let _ = AgentLoop::run(sub_cfg, &sub_provider, sub_tx, None).await;

    // sub_tx is dropped above (moved into AgentLoop::run), so forward_handle will
    // complete when the channel closes.
    let (outcome, summary) = forward_handle.await.unwrap_or_else(|_| {
        ("FAIL".to_string(), "Subagent task failed".to_string())
    });

    // Notify parent TUI that subagent is done.
    if let Some(ref tx) = ctx.event_tx {
        let _ = tx
            .send(ServerMessage::SubagentComplete {
                task: task.clone(),
                outcome: outcome.clone(),
                summary: summary.clone(),
                depth: ctx.depth + 1,
            })
            .await;
    }

    let llm_text = format!(
        "[Tool result: spawn_agent]\nTask: {task}\nWorkspace: {workspace}\nOutcome: {outcome}\nSummary: {summary}"
    );

    if outcome == "PASS" || outcome == "PARTIAL" {
        Ok(ToolOutput::ok(
            format!("Subagent: {outcome}"),
            llm_text,
        ))
    } else {
        Ok(ToolOutput::err(
            format!("Subagent: {outcome}"),
            llm_text,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx(depth: u8) -> ToolContext {
        ToolContext {
            workspace: "/tmp".to_string(),
            on_exec_output: None,
            mcp_clients: None,
            api_key: String::new(),
            model: "claude-sonnet-4-6".to_string(),
            base_url: None,
            event_tx: None,
            depth,
        }
    }

    #[tokio::test]
    async fn spawn_agent_blocks_at_max_depth() {
        let ctx = make_ctx(MAX_DEPTH);
        let args = serde_json::json!({ "task": "do something" });
        let result = spawn_agent(&args, &ctx).await.unwrap();
        assert!(!result.success);
        assert!(result.llm_text.contains("nesting limit"));
    }

    #[tokio::test]
    async fn spawn_agent_requires_task() {
        let ctx = make_ctx(0);
        let args = serde_json::json!({});
        let result = spawn_agent(&args, &ctx).await.unwrap();
        assert!(!result.success);
        assert!(result.llm_text.contains("task"));
    }

    #[test]
    fn max_depth_is_sane() {
        assert_eq!(MAX_DEPTH, 2);
    }
}
