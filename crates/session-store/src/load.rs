//! Session history loader for resume.
//!
//! Reads a `<thread_id>.jsonl` file and reconstructs the provider-facing
//! `Vec<Message>` history so the agent loop can continue from where it
//! left off.
//!
//! ## Reconstruction algorithm
//!
//! Events are grouped into *turns*.  A new turn starts when an `AgentMessage`
//! event is seen.  All `ToolStart` and `ToolResult` events between two
//! consecutive `AgentMessage` events belong to the same turn.  At the end of
//! each turn the buffered data is flushed as:
//!
//!   - `assistant` message: `[optional text block] + [ToolUse blocks from ToolStart events]`
//!   - `user` message:       `[ToolResult blocks from ToolResult events]` (omitted if empty)

use std::path::Path;

use decipher_protocol::ServerMessage;
use decipher_providers::types::{ContentBlock, Message, MessageContent};

use crate::error::StoreError;
use crate::event::SessionMeta;

/// Load a session and reconstruct the provider-facing message history.
///
/// Returns `(meta, messages)` where `messages` is ready to pass as
/// `AgentConfig::resume_from`.
pub async fn load_session(
    base_dir: &Path,
    thread_id: &str,
) -> Result<(SessionMeta, Vec<Message>), StoreError> {
    let file_path = base_dir
        .join("sessions")
        .join(format!("{thread_id}.jsonl"));

    let content = tokio::fs::read_to_string(&file_path).await?;
    let mut lines = content.lines();

    // First line is always the meta header.
    let meta_line = lines.next().ok_or_else(|| {
        StoreError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "session file is empty",
        ))
    })?;
    let meta: SessionMeta = serde_json::from_str(meta_line)?;

    // Collect the `ServerMessage` payloads from all event records.
    let events: Vec<ServerMessage> = lines
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            if v.get("record_type")?.as_str()? != "event" {
                return None;
            }
            serde_json::from_value::<ServerMessage>(v.get("msg")?.clone()).ok()
        })
        .collect();

    let messages = reconstruct_messages(&meta, &events);
    Ok((meta, messages))
}

/// Reconstruct a `Vec<Message>` from the flat event stream.
///
/// Turns are delimited by `AgentMessage` events.  `ToolStart` and
/// `ToolResult` events are buffered within the current turn and flushed
/// as an assistant + user message pair when the next turn begins (or at
/// end-of-stream).
fn reconstruct_messages(meta: &SessionMeta, events: &[ServerMessage]) -> Vec<Message> {
    // Seed with the initial user message that was sent to the LLM at run time.
    let mut messages: Vec<Message> = vec![Message {
        role: "user".into(),
        content: MessageContent::Text(format!(
            "Mission: {}\nWorkspace: {}\n\nBegin. What is your first action?",
            meta.mission_goal, meta.workspace,
        )),
    }];

    // Per-turn accumulators.
    let mut asst_text: Option<String> = None;
    let mut tool_uses: Vec<ContentBlock> = Vec::new();
    let mut tool_results: Vec<ContentBlock> = Vec::new();

    for event in events {
        match event {
            ServerMessage::AgentMessage { text } => {
                // Flush the previous turn before starting the new one.
                flush_turn(&mut messages, &mut asst_text, &mut tool_uses, &mut tool_results);
                if !text.is_empty() {
                    asst_text = Some(text.clone());
                }
            }
            ServerMessage::ToolStart { tool, args, call_id, .. } => {
                tool_uses.push(ContentBlock::ToolUse {
                    id: call_id.clone().unwrap_or_else(|| "unknown".into()),
                    name: tool.clone(),
                    input: args.clone().unwrap_or(serde_json::Value::Object(Default::default())),
                });
            }
            ServerMessage::ToolResult { call_id, success, llm_text, summary, .. } => {
                // Use the stored full LLM text when available; fall back to summary
                // for sessions recorded before `llm_text` was added to the protocol.
                let content = llm_text.clone().unwrap_or_else(|| summary.clone());
                tool_results.push(ContentBlock::ToolResult {
                    tool_use_id: call_id.clone().unwrap_or_default(),
                    content,
                    is_error: !success,
                });
            }
            // All other events (Banner, TokenUsage, FilesModified, MissionComplete, …)
            // are not part of the LLM conversation history.
            _ => {}
        }
    }

    // Flush any remaining partial turn.
    flush_turn(&mut messages, &mut asst_text, &mut tool_uses, &mut tool_results);

    messages
}

/// Emit buffered turn data as messages and reset the accumulators.
fn flush_turn(
    messages: &mut Vec<Message>,
    asst_text: &mut Option<String>,
    tool_uses: &mut Vec<ContentBlock>,
    tool_results: &mut Vec<ContentBlock>,
) {
    // Build assistant message: text block (optional) + tool_use blocks.
    let mut asst_blocks: Vec<ContentBlock> = Vec::new();
    if let Some(text) = asst_text.take() {
        asst_blocks.push(ContentBlock::Text { text });
    }
    asst_blocks.extend(tool_uses.drain(..));
    if !asst_blocks.is_empty() {
        messages.push(Message {
            role: "assistant".into(),
            content: MessageContent::Blocks(asst_blocks),
        });
    }

    // Build user message: tool_result blocks.
    let results: Vec<ContentBlock> = tool_results.drain(..).collect();
    if !results.is_empty() {
        messages.push(Message {
            role: "user".into(),
            content: MessageContent::Blocks(results),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decipher_protocol::ServerMessage;
    use tempfile::TempDir;

    use crate::store::SessionStore;

    fn make_tool_result(call_id: &str, success: bool, text: &str) -> ServerMessage {
        ServerMessage::ToolResult {
            tool: "exec_command".into(),
            success,
            summary: format!("exit {}", if success { 0 } else { 1 }),
            elapsed_ms: 100,
            exit_code: Some(if success { 0 } else { 1 }),
            output_preview: None,
            output_lines_total: None,
            call_id: Some(call_id.into()),
            llm_text: Some(text.into()),
            parsed_output: None,
        }
    }

    #[tokio::test]
    async fn round_trip_single_turn() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path(), "test-model", "/work", "build the app")
            .await
            .unwrap();
        let thread_id = store.thread_id().to_string();

        store.record(&ServerMessage::AgentMessage { text: "I'll run make".into() });
        store.record(&ServerMessage::ToolStart {
            tool: "exec_command".into(),
            reasoning: "build".into(),
            args: Some(serde_json::json!({"cmd": "make"})),
            call_id: Some("c1".into()),
        });
        store.record(&make_tool_result("c1", true, "Build succeeded"));
        store.close(Some("PASS".into())).await;

        let (meta, messages) = load_session(dir.path(), &thread_id).await.unwrap();
        assert_eq!(meta.mission_goal, "build the app");

        // Expected: [initial_user, assistant(text + tool_use), user(tool_result)]
        assert_eq!(messages.len(), 3, "messages: {messages:?}");

        // [0] initial user message
        assert!(matches!(messages[0].content, MessageContent::Text(_)));

        // [1] assistant: text + tool_use
        if let MessageContent::Blocks(ref blocks) = messages[1].content {
            assert_eq!(blocks.len(), 2);
            assert!(matches!(blocks[0], ContentBlock::Text { .. }));
            assert!(matches!(blocks[1], ContentBlock::ToolUse { .. }));
        } else {
            panic!("expected Blocks for assistant message");
        }

        // [2] user: tool_result
        if let MessageContent::Blocks(ref blocks) = messages[2].content {
            assert_eq!(blocks.len(), 1);
            if let ContentBlock::ToolResult { content, is_error, .. } = &blocks[0] {
                assert_eq!(content, "Build succeeded");
                assert!(!is_error);
            } else {
                panic!("expected ToolResult block");
            }
        } else {
            panic!("expected Blocks for user tool-result message");
        }
    }

    #[tokio::test]
    async fn round_trip_two_turns() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path(), "m", "/w", "fix tests")
            .await
            .unwrap();
        let thread_id = store.thread_id().to_string();

        // Turn 1
        store.record(&ServerMessage::AgentMessage { text: "check".into() });
        store.record(&ServerMessage::ToolStart {
            tool: "read_file".into(),
            reasoning: "".into(),
            args: Some(serde_json::json!({"path": "src/main.rs"})),
            call_id: Some("t1".into()),
        });
        store.record(&make_tool_result("t1", true, "fn main() {}"));

        // Turn 2
        store.record(&ServerMessage::AgentMessage { text: "all good".into() });
        store.close(None).await;

        let (_, messages) = load_session(dir.path(), &thread_id).await.unwrap();
        // initial_user + asst1 + user1 + asst2 = 4
        assert_eq!(messages.len(), 4, "messages: {messages:?}");
        assert_eq!(messages[3].role, "assistant");
    }

    #[tokio::test]
    async fn missing_session_returns_error() {
        let dir = TempDir::new().unwrap();
        let result = load_session(dir.path(), "nonexistent-uuid").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn falls_back_to_summary_when_llm_text_absent() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path(), "m", "/w", "goal").await.unwrap();
        let thread_id = store.thread_id().to_string();

        store.record(&ServerMessage::AgentMessage { text: "ok".into() });
        store.record(&ServerMessage::ToolStart {
            tool: "exec_command".into(),
            reasoning: "".into(),
            args: None,
            call_id: Some("x1".into()),
        });
        // Old-format ToolResult without llm_text
        store.record(&ServerMessage::ToolResult {
            tool: "exec_command".into(),
            success: true,
            summary: "exit 0".into(),
            elapsed_ms: 10,
            exit_code: Some(0),
            output_preview: None,
            output_lines_total: None,
            call_id: Some("x1".into()),
            llm_text: None,
            parsed_output: None,
        });
        store.close(None).await;

        let (_, messages) = load_session(dir.path(), &thread_id).await.unwrap();
        if let Some(MessageContent::Blocks(ref blocks)) = messages.get(2).map(|m| &m.content) {
            if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
                assert_eq!(content, "exit 0", "should fall back to summary");
            } else {
                panic!("expected ToolResult block");
            }
        } else {
            panic!("expected user message with blocks");
        }
    }
}
