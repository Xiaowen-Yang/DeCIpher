//! Rust-native agent loop — replaces `agents/executor/agent-loop.js`.
//!
//! # Execution model
//!
//! 1. Build system prompt from `AgentConfig`.
//! 2. Call the provider with the current message history + tool definitions.
//! 3. For each tool_use block in the response:
//!    a. Emit `ToolStart` event.
//!    b. Check policy (Allow / Deny / Ask).
//!    c. Execute tool → get `ToolOutput`.
//!    d. Emit `ToolResult` event.
//!    e. Append tool_result message to history.
//! 4. If the `done` tool was called: emit `MissionComplete`, return.
//! 5. Compact if approaching context window limit.
//! 6. Repeat up to `max_turns`.

use std::time::Instant;

use decipher_policy::{Decision, PermissionAmendments, evaluate_policy, record_approval};
use decipher_protocol::ServerMessage;
use decipher_providers::{
    Provider,
    types::{
        ContentBlock, ContentDelta, Message, MessageContent, MessageRequest, StreamEvent,
        ToolDefinition, TokenUsage,
    },
};
use decipher_tools::spec::all_tool_specs;
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::{
    RuntimeError,
    compaction::{compact_messages, should_compact},
    tools::{ToolContext, dispatch},
    types::{AgentConfig, RunOutcome, RunResult},
};

const MAX_NO_TOOL_CALLS: u32 = 3;
const COMPACT_KEEP_RECENT: usize = 6;

/// The agent loop entry point.
pub struct AgentLoop;

impl AgentLoop {
    /// Run the agent loop until completion or error.
    ///
    /// # Parameters
    /// - `config`: mission configuration (model, workspace, goal, policy)
    /// - `provider`: LLM provider (Anthropic or mock)
    /// - `event_tx`: channel for emitting `ServerMessage` events to the TUI
    /// - `approval_rx`: optional channel for receiving approval decisions
    pub async fn run(
        config: AgentConfig,
        provider: &dyn Provider,
        event_tx: mpsc::Sender<ServerMessage>,
        mut approval_rx: Option<mpsc::Receiver<bool>>,
    ) -> Result<RunResult, RuntimeError> {
        let start = Instant::now();
        let model_info = provider.model_info();
        let context_window = model_info.context_window;

        // Announce the run.
        let _ = event_tx
            .send(ServerMessage::Banner {
                version: env!("CARGO_PKG_VERSION").to_string(),
                provider: "anthropic".to_string(),
                model: config.model.clone(),
                directory: config.workspace.clone(),
                api_key_set: !config.api_key.is_empty(),
            })
            .await;

        // Build tool definitions for the LLM.
        let tools: Vec<ToolDefinition> = all_tool_specs()
            .into_iter()
            .map(|s| ToolDefinition {
                name: s.name.to_string(),
                description: Some(s.description.to_string()),
                input_schema: Some(s.input_schema),
            })
            .collect();

        // Build the initial message history.
        let system_prompt = build_system_prompt(&config);
        let mut messages: Vec<Message> = vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text(build_initial_user_message(&config)),
        }];

        let mut amendments = PermissionAmendments::new();
        let tool_ctx = ToolContext {
            workspace: config.workspace.clone(),
            on_exec_output: None,
        };

        let mut outcome = RunOutcome::Fail;
        let mut final_summary = String::from("Agent loop completed without a done call.");
        let mut done_result: Option<DoneResult> = None;
        let mut consecutive_no_tools = 0u32;
        let mut turns_completed = 0u32;
        let mut last_prompt_tokens = 0u32;

        // ── Main turn loop ────────────────────────────────────────────────────
        for turn in 1..=config.max_turns {
            turns_completed = turn;

            // Emit status update.
            let _ = event_tx
                .send(ServerMessage::AgentStatus {
                    phase: "thinking".to_string(),
                    turn,
                    max_turns: config.max_turns,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    tool_name: None,
                })
                .await;

            // ── Call provider (streaming) ─────────────────────────────────────
            let request = MessageRequest {
                model: config.model.clone(),
                messages: messages.clone(),
                tools: Some(tools.clone()),
                max_tokens: config.max_tokens,
                stream: true,
                system: Some(system_prompt.clone()),
            };

            let mut stream = provider.stream_message(request).await?;

            // Collect streaming response.
            let collected = collect_stream(&mut stream, &event_tx).await?;
            last_prompt_tokens = collected.usage.input_tokens;

            // Emit token usage.
            let _ = event_tx
                .send(ServerMessage::TokenUsage {
                    prompt_tokens: collected.usage.input_tokens as u64,
                    completion_tokens: collected.usage.output_tokens as u64,
                    total_tokens: (collected.usage.input_tokens
                        + collected.usage.output_tokens) as u64,
                    context_window: Some(context_window as u64),
                })
                .await;

            // ── Handle no-tool-call turns ─────────────────────────────────────
            let has_tools = collected
                .content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { .. }));

            if !has_tools {
                consecutive_no_tools += 1;
                if consecutive_no_tools >= MAX_NO_TOOL_CALLS {
                    final_summary = format!(
                        "Agent failed to produce tool calls after {MAX_NO_TOOL_CALLS} attempts."
                    );
                    break;
                }
                // Add the assistant text to history and nudge.
                if let Some(text) = first_text(&collected.content) {
                    messages.push(Message {
                        role: "assistant".to_string(),
                        content: MessageContent::Text(text),
                    });
                }
                messages.push(Message {
                    role: "user".to_string(),
                    content: MessageContent::Text(
                        "You must use one of the available tools to make progress. \
                         Call a tool now to continue working on the mission."
                            .to_string(),
                    ),
                });
                continue;
            }
            consecutive_no_tools = 0;

            // Push the full assistant message (text + tool_use blocks) to history.
            messages.push(Message {
                role: "assistant".to_string(),
                content: MessageContent::Blocks(collected.content.clone()),
            });

            // ── Process tool calls ────────────────────────────────────────────
            let mut done_in_turn = false;
            let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

            let reasoning = first_text(&collected.content).unwrap_or_default();

            for block in &collected.content {
                let ContentBlock::ToolUse { id, name, input } = block else {
                    continue;
                };

                // Handle `done` before anything else.
                if name == "done" {
                    let dr = parse_done_result(input);
                    outcome = RunOutcome::from_str(&dr.outcome);
                    final_summary = dr.summary.clone();

                    let elapsed = start.elapsed().as_millis() as u64;
                    let _ = event_tx
                        .send(ServerMessage::MissionComplete {
                            outcome: outcome.as_str().to_string(),
                            summary: final_summary.clone(),
                            turns: turns_completed,
                            elapsed_ms: elapsed,
                            urls: Vec::new(),
                            files_modified: dr.files_modified.clone(),
                            errors_encountered: dr.errors_encountered.clone(),
                            next_steps: dr.next_steps.clone(),
                        })
                        .await;
                    done_result = Some(dr);
                    done_in_turn = true;
                    break;
                }

                // Emit ToolStart.
                let _ = event_tx
                    .send(ServerMessage::ToolStart {
                        tool: name.clone(),
                        reasoning: reasoning.chars().take(200).collect(),
                        args: Some(input.clone()),
                        call_id: Some(id.clone()),
                    })
                    .await;

                // Policy check.
                let policy_result = evaluate_policy(
                    config.policy_mode,
                    name,
                    input,
                    &amendments,
                    Some(config.workspace.as_str()),
                );

                let tool_class = policy_result.tool_class;
                let tool_result_text: String;

                match policy_result.decision {
                    Decision::Deny => {
                        tool_result_text = format!(
                            "Error: Action denied by policy ({}). \
                             Try a different approach that does not require {} access.",
                            policy_result.reason, tool_class
                        );
                        tool_result_blocks.push(ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: tool_result_text,
                            is_error: true,
                        });
                    }
                    Decision::Ask => {
                        // Wait for approval from the TUI, or auto-approve if no channel.
                        let approved = if let Some(rx) = approval_rx.as_mut() {
                            let _ = event_tx
                                .send(ServerMessage::ApprovalRequest {
                                    capabilities: vec![tool_class.to_string()],
                                    action: Some(decipher_protocol::ActionDetail {
                                        tool: name.clone(),
                                        reasoning: Some(reasoning.chars().take(200).collect()),
                                    }),
                                })
                                .await;
                            rx.recv().await.unwrap_or(false)
                        } else {
                            true // No approval channel — auto-approve in non-interactive mode.
                        };

                        if !approved {
                            outcome = RunOutcome::Fail;
                            final_summary =
                                "Stopped: approval denied for risky operation.".to_string();
                            done_in_turn = true;
                            break;
                        }

                        record_approval(&mut amendments, tool_class, Some(name.as_str()));

                        // Execute the tool.
                        let (result_block, emitted_done) = execute_tool_and_emit(
                            name,
                            id,
                            input,
                            &tool_ctx,
                            &event_tx,
                            start,
                        )
                        .await?;
                        tool_result_blocks.push(result_block);
                        if emitted_done {
                            done_in_turn = true;
                            break;
                        }
                    }
                    Decision::Allow => {
                        let (result_block, emitted_done) = execute_tool_and_emit(
                            name,
                            id,
                            input,
                            &tool_ctx,
                            &event_tx,
                            start,
                        )
                        .await?;
                        tool_result_blocks.push(result_block);
                        if emitted_done {
                            done_in_turn = true;
                            break;
                        }
                    }
                }
            }

            if done_in_turn {
                break;
            }

            // Push tool results to history.
            if !tool_result_blocks.is_empty() {
                messages.push(Message {
                    role: "user".to_string(),
                    content: MessageContent::Blocks(tool_result_blocks),
                });
            }

            // ── Compaction ────────────────────────────────────────────────────
            if should_compact(last_prompt_tokens, context_window) && messages.len() > 8 {
                messages = compact_messages(&messages, COMPACT_KEEP_RECENT);
            }
        }

        let elapsed_ms = start.elapsed().as_millis() as u64;

        Ok(RunResult {
            outcome,
            summary: final_summary,
            turns_completed,
            elapsed_ms,
            files_modified: done_result
                .as_ref()
                .map(|d| d.files_modified.clone())
                .unwrap_or_default(),
            errors_encountered: done_result
                .as_ref()
                .map(|d| d.errors_encountered.clone())
                .unwrap_or_default(),
            next_steps: done_result
                .as_ref()
                .map(|d| d.next_steps.clone())
                .unwrap_or_default(),
        })
    }
}

// ── Tool execution ────────────────────────────────────────────────────────────

/// Execute a single tool call and emit ToolResult + FilesModified events.
/// Returns the tool_result ContentBlock to push into history, and whether done
/// should be signaled.
async fn execute_tool_and_emit(
    name: &str,
    call_id: &str,
    input: &serde_json::Value,
    tool_ctx: &ToolContext,
    event_tx: &mpsc::Sender<ServerMessage>,
    loop_start: Instant,
) -> Result<(ContentBlock, bool), RuntimeError> {
    let exec_start = Instant::now();
    let tool_output = dispatch(name, input, tool_ctx).await?;
    let elapsed_ms = exec_start.elapsed().as_millis() as u64;

    // Emit ToolResult.
    let _ = event_tx
        .send(ServerMessage::ToolResult {
            tool: name.to_string(),
            success: tool_output.success,
            summary: tool_output.summary.clone(),
            elapsed_ms,
            exit_code: tool_output.exit_code,
            output_preview: tool_output.raw_output.as_ref().and_then(|o| {
                let lines: Vec<&str> = o.lines().collect();
                if lines.len() > 8 {
                    Some(lines[..8].join("\n"))
                } else {
                    None
                }
            }),
            output_lines_total: tool_output
                .raw_output
                .as_deref()
                .map(|o| o.lines().count() as u32),
            call_id: Some(call_id.to_string()),
        })
        .await;

    // Emit FilesModified for write_file / apply_patch.
    if tool_output.success && (name == "write_file" || name == "apply_patch") {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| {
                // For apply_patch, extract from the patch header.
                input
                    .get("target_file")
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("(unknown)")
            .to_string();

        let _ = event_tx
            .send(ServerMessage::FilesModified {
                files: vec![decipher_protocol::FileModification {
                    path,
                    added: None,
                    removed: None,
                    preview: Vec::new(),
                }],
            })
            .await;
    }

    let _ = loop_start; // used for potential future timing

    let is_error = !tool_output.success;
    let block = ContentBlock::ToolResult {
        tool_use_id: call_id.to_string(),
        content: tool_output.llm_text.clone(),
        is_error,
    };

    Ok((block, false))
}

// ── Streaming response collector ──────────────────────────────────────────────

struct CollectedResponse {
    content: Vec<ContentBlock>,
    usage: TokenUsage,
}

async fn collect_stream(
    stream: &mut (impl futures::Stream<Item = decipher_providers::Result<StreamEvent>> + Unpin),
    event_tx: &mpsc::Sender<ServerMessage>,
) -> Result<CollectedResponse, RuntimeError> {
    let mut content_blocks: Vec<ContentBlock> = Vec::new();
    let mut usage = TokenUsage::default();

    // Accumulated JSON for in-progress tool_use input_json_delta events.
    // Key = block index (u32), value = accumulated partial JSON.
    let mut tool_json_accumulator: std::collections::HashMap<u32, String> =
        std::collections::HashMap::new();

    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::MessageStart { usage: u, .. } => {
                usage.input_tokens = u.input_tokens;
            }
            StreamEvent::ContentBlockStart {
                index,
                content_block,
            } => {
                // Grow block list to accommodate out-of-order indices.
                while content_blocks.len() <= index as usize {
                    content_blocks.push(ContentBlock::Text {
                        text: String::new(),
                    });
                }
                content_blocks[index as usize] = content_block;
            }
            StreamEvent::ContentBlockDelta { index, delta } => match delta {
                ContentDelta::TextDelta { text } => {
                    // Emit streaming text to TUI.
                    let _ = event_tx
                        .send(ServerMessage::AgentMessageDelta {
                            delta: text.clone(),
                        })
                        .await;
                    // Append to the text block.
                    if let Some(ContentBlock::Text { text: t }) =
                        content_blocks.get_mut(index as usize)
                    {
                        t.push_str(&text);
                    }
                }
                ContentDelta::InputJsonDelta { partial_json } => {
                    tool_json_accumulator
                        .entry(index)
                        .or_default()
                        .push_str(&partial_json);
                }
            },
            StreamEvent::ContentBlockStop { index } => {
                // Finalise tool_use input if we have accumulated JSON.
                if let Some(json) = tool_json_accumulator.remove(&index) {
                    if let Some(ContentBlock::ToolUse { input, .. }) =
                        content_blocks.get_mut(index as usize)
                    {
                        *input = serde_json::from_str(&json)
                            .unwrap_or(serde_json::Value::String(json));
                    }
                }
            }
            StreamEvent::MessageDelta { usage: u, .. } => {
                usage.output_tokens = u.output_tokens;
            }
            StreamEvent::MessageStop => {}
        }
    }

    // Flush any remaining unfinished tool JSON (safety net).
    for (index, json) in tool_json_accumulator {
        if let Some(ContentBlock::ToolUse { input, .. }) =
            content_blocks.get_mut(index as usize)
        {
            *input = serde_json::from_str(&json)
                .unwrap_or(serde_json::Value::String(json));
        }
    }

    // Remove placeholder text blocks that ended up empty.
    content_blocks.retain(|b| match b {
        ContentBlock::Text { text } => !text.is_empty(),
        _ => true,
    });

    Ok(CollectedResponse {
        content: content_blocks,
        usage,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn first_text(blocks: &[ContentBlock]) -> Option<String> {
    blocks.iter().find_map(|b| {
        if let ContentBlock::Text { text } = b {
            if !text.is_empty() {
                return Some(text.clone());
            }
        }
        None
    })
}

struct DoneResult {
    outcome: String,
    summary: String,
    files_modified: Vec<String>,
    errors_encountered: Vec<String>,
    next_steps: Vec<String>,
}

fn parse_done_result(input: &serde_json::Value) -> DoneResult {
    DoneResult {
        outcome: input
            .get("outcome")
            .and_then(|v| v.as_str())
            .unwrap_or("FAIL")
            .to_string(),
        summary: input
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("Mission complete.")
            .to_string(),
        files_modified: extract_string_array(input, "files_modified"),
        errors_encountered: extract_string_array(input, "errors_encountered"),
        next_steps: extract_string_array(input, "next_steps"),
    }
}

fn extract_string_array(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn build_system_prompt(config: &AgentConfig) -> String {
    let steps_text = if config.plan_steps.is_empty() {
        "(determine the steps yourself based on the goal)".to_string()
    } else {
        config
            .plan_steps
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{}. {}", i + 1, s))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "You are DeCIpher, a mission-driven local execution agent.\n\n\
         ## Mission\n\
         Goal: {mission_goal}\n\
         Working directory: {workspace}\n\n\
         ## Plan\n\
         {steps}\n\n\
         ## Environment\n\
         Host OS: {os}\n\n\
         ## Instructions\n\
         Use the available tools to accomplish the mission. \
         Call `done` only when the user's goal is verified as satisfied.\n\
         When you call `done`, set outcome to PASS if the goal was achieved, \
         FAIL if it could not be, or PARTIAL if some steps succeeded but the goal is not fully met.",
        mission_goal = config.mission_goal,
        workspace = config.workspace,
        steps = steps_text,
        os = std::env::consts::OS,
    )
}

fn build_initial_user_message(config: &AgentConfig) -> String {
    format!(
        "Mission: {}\nWorkspace: {}\n\nBegin. What is your first action?",
        config.mission_goal, config.workspace
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_contains_goal() {
        let config = AgentConfig {
            mission_goal: "Fix the CI pipeline".to_string(),
            workspace: "/workspace".to_string(),
            ..Default::default()
        };
        let prompt = build_system_prompt(&config);
        assert!(prompt.contains("Fix the CI pipeline"));
        assert!(prompt.contains("/workspace"));
    }

    #[test]
    fn system_prompt_with_steps() {
        let config = AgentConfig {
            mission_goal: "Build docker image".to_string(),
            workspace: "/workspace".to_string(),
            plan_steps: vec!["Step 1: clone".to_string(), "Step 2: build".to_string()],
            ..Default::default()
        };
        let prompt = build_system_prompt(&config);
        assert!(prompt.contains("1. Step 1: clone"));
        assert!(prompt.contains("2. Step 2: build"));
    }

    #[test]
    fn parse_done_extracts_all_fields() {
        let input = serde_json::json!({
            "outcome": "PASS",
            "summary": "All tests passed",
            "files_modified": ["src/main.rs"],
            "errors_encountered": [],
            "next_steps": ["deploy"]
        });
        let dr = parse_done_result(&input);
        assert_eq!(dr.outcome, "PASS");
        assert_eq!(dr.summary, "All tests passed");
        assert_eq!(dr.files_modified, vec!["src/main.rs"]);
        assert_eq!(dr.next_steps, vec!["deploy"]);
    }

    #[test]
    fn run_outcome_round_trips() {
        assert_eq!(RunOutcome::from_str("PASS"), RunOutcome::Pass);
        assert_eq!(RunOutcome::from_str("FAIL"), RunOutcome::Fail);
        assert_eq!(RunOutcome::from_str("PARTIAL"), RunOutcome::Partial);
        assert_eq!(RunOutcome::from_str("unknown"), RunOutcome::Fail);
    }

    #[test]
    fn first_text_returns_non_empty() {
        let blocks = vec![
            ContentBlock::Text { text: String::new() },
            ContentBlock::Text { text: "hello".to_string() },
        ];
        assert_eq!(first_text(&blocks), Some("hello".to_string()));
    }

    #[test]
    fn first_text_returns_none_for_empty_blocks() {
        let blocks: Vec<ContentBlock> = vec![];
        assert_eq!(first_text(&blocks), None);
    }
}
