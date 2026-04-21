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

use async_recursion::async_recursion;
use decipher_policy::{Decision, PermissionAmendments, evaluate_policy, record_approval};
use decipher_protocol::ServerMessage;
use decipher_providers::{
    Provider,
    types::{
        ContentBlock, ContentDelta, Message, MessageContent, MessageRequest, StreamEvent,
        ToolDefinition, TokenUsage,
    },
};
use decipher_tools::classify::is_read_only_by_name;
use decipher_tools::spec::all_tool_specs;
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::{
    RuntimeError,
    compaction::{compact_messages, should_compact},
    hooks::{HookConfig, fire_post_tool_use, fire_pre_tool_use, fire_session_event},
    skills::format_skills_section,
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
    ///
    /// This function is indirectly recursive via spawn_agent → AgentLoop::run.
    /// The `#[async_recursion]` attribute boxes the future to break the cycle.
    #[async_recursion]
    #[allow(unused_assignments)]
    pub async fn run(
        config: AgentConfig,
        provider: &dyn Provider,
        event_tx: mpsc::Sender<ServerMessage>,
        mut approval_rx: Option<mpsc::Receiver<bool>>,
    ) -> Result<RunResult, RuntimeError> {
        let start = Instant::now();
        let model_info = provider.model_info();
        let context_window = model_info.context_window;

        // Generate a session identifier for hooks.
        // Using SystemTime (milliseconds since epoch) gives a unique, monotonic id.
        let session_id = format!(
            "session-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );

        // Fire session start hooks.
        fire_session_event(&config.hook_config.session_start).await;

        // Build tool definitions for the LLM.
        // In plan_mode, no tools are provided so the LLM generates text only.
        let tools: Vec<ToolDefinition> = if config.plan_mode {
            Vec::new()
        } else {
            let mut tool_defs: Vec<ToolDefinition> = all_tool_specs()
                .into_iter()
                .map(|s| ToolDefinition {
                    name: s.name.to_string(),
                    description: Some(s.description.to_string()),
                    input_schema: Some(s.input_schema),
                })
                .collect();
            // Merge MCP tools — prefixed as mcp__<server>_<tool> to avoid collisions
            // with built-in tools (e.g. a server's "read_file" won't shadow the built-in).
            for mcp_tool in &config.mcp_tools {
                tool_defs.push(ToolDefinition {
                    name: format!("mcp__{}_{}", mcp_tool.server_name, mcp_tool.name),
                    description: if mcp_tool.description.is_empty() {
                        None
                    } else {
                        Some(format!("[MCP:{}] {}", mcp_tool.server_name, mcp_tool.description))
                    },
                    input_schema: {
                        // 3D: validate schema is an object so the Anthropic API doesn't 400.
                        let schema = &mcp_tool.input_schema;
                        if schema.is_object() {
                            Some(schema.clone())
                        } else {
                            Some(serde_json::json!({"type": "object", "properties": {}}))
                        }
                    },
                });
            }
            tool_defs
        };

        // Build the initial message history (or restore from a resumed session).
        let system_prompt = build_system_prompt(&config);
        let mut messages: Vec<Message> = if let Some(history) = config.resume_from.clone() {
            history
        } else {
            vec![Message {
                role: "user".to_string(),
                content: MessageContent::Text(build_initial_user_message(&config)),
            }]
        };

        let mut amendments = PermissionAmendments::new();

        // Wire exec output streaming to TUI via ExecOutputDelta events.
        let (exec_out_tx, mut exec_out_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        {
            let event_tx_clone = event_tx.clone();
            tokio::spawn(async move {
                while let Some(line) = exec_out_rx.recv().await {
                    let _ = event_tx_clone
                        .send(ServerMessage::ExecOutputDelta { delta: line })
                        .await;
                }
            });
        }

        let tool_ctx = ToolContext {
            workspace: config.workspace.clone(),
            on_exec_output: Some(exec_out_tx),
            mcp_clients: config.mcp_clients.clone(),
            api_key: config.api_key.clone(),
            model: config.model.clone(),
            base_url: config.base_url.clone(),
            event_tx: Some(event_tx.clone()),
            depth: config.depth,
            policy_mode: config.policy_mode,
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
                tools: if config.plan_mode { None } else { Some(tools.clone()) },
                max_tokens: config.max_tokens,
                stream: true,
                system: Some(system_prompt.clone()),
            };

            let mut stream = match provider.stream_message(request).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[agent] provider error: {e}");
                    let _ = event_tx.send(ServerMessage::Error {
                        message: format!("Provider error: {e}"),
                    }).await;
                    return Err(e.into());
                }
            };

            // Collect streaming response.
            let collected = match collect_stream(&mut stream, &event_tx).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[agent] stream error: {e}");
                    let _ = event_tx.send(ServerMessage::Error {
                        message: format!("Stream error: {e}"),
                    }).await;
                    return Err(e);
                }
            };
            last_prompt_tokens = collected.usage.input_tokens;

            // Emit the assembled assistant text as AgentMessage so the session
            // store can record it for history reconstruction.
            if let Some(text) = first_text(&collected.content) {
                if !text.is_empty() {
                    let _ = event_tx
                        .send(ServerMessage::AgentMessage { text })
                        .await;
                }
            }

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
                // In plan_mode, a text-only response IS the expected output.
                if config.plan_mode {
                    let plan_text =
                        first_text(&collected.content).unwrap_or_else(|| "(no plan generated)".to_string());
                    outcome = RunOutcome::Fail; // placeholder — CLI overrides for PLAN
                    final_summary = plan_text.clone();
                    let elapsed = start.elapsed().as_millis() as u64;
                    let _ = event_tx
                        .send(ServerMessage::MissionComplete {
                            outcome: "PLAN".to_string(),
                            summary: plan_text,
                            turns: turns_completed,
                            elapsed_ms: elapsed,
                            urls: Vec::new(),
                            files_modified: Vec::new(),
                            errors_encountered: Vec::new(),
                            next_steps: Vec::new(),
                        })
                        .await;
                    break;
                }

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

            // Classified tool calls for this turn.
            struct PendingTool {
                id: String,
                name: String,
                input: serde_json::Value,
                decision: Decision,
                tool_class: decipher_policy::ToolClass,
                reason: String,
            }

            let mut parallel_tools: Vec<PendingTool> = Vec::new();
            let mut sequential_tools: Vec<PendingTool> = Vec::new();

            // First pass: handle `done` immediately; classify remaining tools.
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

                // Evaluate policy early for classification.
                let policy_result = evaluate_policy(
                    config.policy_mode,
                    name,
                    input,
                    &amendments,
                    Some(config.workspace.as_str()),
                );

                let pending = PendingTool {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                    decision: policy_result.decision,
                    tool_class: policy_result.tool_class,
                    reason: policy_result.reason.to_string(),
                };

                // Parallel: read-only AND allowed (no user gate, no risk).
                if is_read_only_by_name(name) && policy_result.decision == Decision::Allow {
                    parallel_tools.push(pending);
                } else {
                    sequential_tools.push(pending);
                }
            }

            if done_in_turn {
                // Already handled above.
            } else {
                // ── Parallel batch ────────────────────────────────────────────
                if !parallel_tools.is_empty() {
                    // Emit ToolStart for each parallel tool.
                    for t in &parallel_tools {
                        let _ = event_tx
                            .send(ServerMessage::ToolStart {
                                tool: t.name.clone(),
                                reasoning: reasoning.chars().take(200).collect(),
                                args: Some(t.input.clone()),
                                call_id: Some(t.id.clone()),
                            })
                            .await;
                    }

                    // Execute all parallel tools concurrently.
                    let futs = parallel_tools.iter().map(|t| {
                        let name = t.name.clone();
                        let id = t.id.clone();
                        let input = t.input.clone();
                        let ctx = tool_ctx.clone();
                        let tx = event_tx.clone();
                        let hc = config.hook_config.clone();
                        let sid = session_id.clone();
                        async move {
                            execute_tool_and_emit(&name, &id, &input, &ctx, &tx, start, &hc, &sid).await
                        }
                    });
                    let results = futures::future::join_all(futs).await;
                    for result in results {
                        let (block, _emitted_done) = result?;
                        tool_result_blocks.push(block);
                    }
                }

                // ── Sequential tools ──────────────────────────────────────────
                for t in &sequential_tools {
                    // Emit ToolStart.
                    let _ = event_tx
                        .send(ServerMessage::ToolStart {
                            tool: t.name.clone(),
                            reasoning: reasoning.chars().take(200).collect(),
                            args: Some(t.input.clone()),
                            call_id: Some(t.id.clone()),
                        })
                        .await;

                    let tool_result_text: String;
                    match t.decision {
                        Decision::Deny => {
                            tool_result_text = format!(
                                "Error: Action denied by policy ({}). \
                                 Try a different approach that does not require {} access.",
                                t.reason, t.tool_class
                            );
                            tool_result_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: t.id.clone(),
                                content: tool_result_text,
                                is_error: true,
                            });
                        }
                        Decision::Ask => {
                            // Wait for approval from the TUI, or auto-approve if no channel.
                            let approved = if let Some(rx) = approval_rx.as_mut() {
                                let _ = event_tx
                                    .send(ServerMessage::ApprovalRequest {
                                        capabilities: vec![t.tool_class.to_string()],
                                        action: Some(decipher_protocol::ActionDetail {
                                            tool: t.name.clone(),
                                            reasoning: Some(
                                                reasoning.chars().take(200).collect(),
                                            ),
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

                            record_approval(&mut amendments, t.tool_class, Some(t.name.as_str()));

                            let (result_block, emitted_done) = execute_tool_and_emit(
                                &t.name,
                                &t.id,
                                &t.input,
                                &tool_ctx,
                                &event_tx,
                                start,
                                &config.hook_config,
                                &session_id,
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
                                &t.name,
                                &t.id,
                                &t.input,
                                &tool_ctx,
                                &event_tx,
                                start,
                                &config.hook_config,
                                &session_id,
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

        // Fire session end hooks.
        fire_session_event(&config.hook_config.session_end).await;

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
    hook_config: &HookConfig,
    session_id: &str,
) -> Result<(ContentBlock, bool), RuntimeError> {
    // Fire PreToolUse hooks — if blocked, return a synthetic error result.
    let pre_result = fire_pre_tool_use(hook_config, name, input, session_id).await;
    if pre_result.blocked {
        let err_text = format!(
            "Error: tool call blocked by PreToolUse hook: {}",
            pre_result.reason
        );
        let block = ContentBlock::ToolResult {
            tool_use_id: call_id.to_string(),
            content: err_text.clone(),
            is_error: true,
        };
        // Emit ToolResult for the TUI to display.
        let _ = event_tx
            .send(ServerMessage::ToolResult {
                tool: name.to_string(),
                success: false,
                summary: format!("Blocked: {}", pre_result.reason),
                elapsed_ms: 0,
                exit_code: None,
                output_preview: None,
                output_lines_total: None,
                call_id: Some(call_id.to_string()),
                llm_text: Some(err_text),
                parsed_output: None,
            })
            .await;
        return Ok((block, false));
    }

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
            output_preview: tool_output.raw_output.clone(),
            output_lines_total: tool_output
                .raw_output
                .as_deref()
                .map(|o| o.lines().count() as u32),
            call_id: Some(call_id.to_string()),
            // Store full LLM-facing text for lossless session resume reconstruction.
            llm_text: Some(tool_output.llm_text.clone()),
            // JSON-serialized smart-card data for TUI rendering (display only).
            parsed_output: tool_output.parsed_output.as_ref()
                .and_then(|p| serde_json::to_string(p).ok()),
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

    // Fire PostToolUse hooks (best-effort).
    fire_post_tool_use(
        hook_config,
        name,
        tool_output.success,
        &tool_output.summary,
        tool_output.exit_code,
    )
    .await;

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

    let mut prompt = format!(
        "You are DeCIpher, a mission-driven local execution agent.\n\n\
         ## Mission\n\
         Goal: {mission_goal}\n\
         Working directory: {workspace}\n\n\
         ## Plan\n\
         {steps}\n\n\
         ## Environment\n\
         Host OS: {os}\n\n",
        mission_goal = config.mission_goal,
        workspace = config.workspace,
        steps = steps_text,
        os = std::env::consts::OS,
    );

    // Inject memory context if available.
    if let Some(ref mem) = config.memory_context {
        if !mem.is_empty() {
            prompt.push_str("## Memory\n");
            prompt.push_str(mem);
            prompt.push_str("\n\n");
        }
    }

    // Inject skills if available.
    let skills_section = format_skills_section(&config.skills);
    if !skills_section.is_empty() {
        prompt.push_str(&skills_section);
        prompt.push_str("\n\n");
    }

    if config.plan_mode {
        prompt.push_str(
            "## Instructions\n\
             You are in PLAN MODE. Generate a step-by-step plan for the mission. \
             Do NOT call any tools. Describe your approach in numbered steps. \
             Include what tools you would use at each step and what you expect to verify.",
        );
    } else {
        prompt.push_str(
            "## Instructions\n\
             Use the available tools to accomplish the mission. \
             Call `done` only when the user's goal is verified as satisfied.\n\
             When you call `done`, set outcome to PASS if the goal was achieved, \
             FAIL if it could not be, or PARTIAL if some steps succeeded but the goal is not fully met.",
        );
    }

    prompt
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
    use decipher_providers::anthropic::AnthropicProvider;

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

    /// Verify that multiple read-only tools in one turn all produce results.
    /// Uses the MockProviderService to serve a multi_tool_turn scenario
    /// (two read_file calls), then runs one turn of the AgentLoop and checks
    /// that two ToolResult events arrived — proving the parallel batch ran.
    #[tokio::test]
    async fn parallel_read_only_tools_both_produce_results() {
        use decipher_mock_provider::MockProviderService;
        use tokio::sync::mpsc;

        let mock = MockProviderService::spawn().await.unwrap();
        let provider = AnthropicProvider::new("test-key", "claude-sonnet-4-5-20250514")
            .with_base_url(mock.base_url());

        let (tx, mut rx) = mpsc::channel::<ServerMessage>(64);

        let workspace = tempfile::tempdir().unwrap();
        // Create the files the mock tool will try to read.
        std::fs::write(workspace.path().join("a.txt"), "content_a").unwrap();
        std::fs::write(workspace.path().join("b.txt"), "content_b").unwrap();

        let cfg = crate::types::AgentConfig {
            model: "claude-sonnet-4-5-20250514".to_string(),
            api_key: "test-key".to_string(),
            workspace: workspace.path().to_string_lossy().to_string(),
            mission_goal: "PARITY_SCENARIO:multi_tool_turn read a.txt b.txt".to_string(),
            max_turns: 3,
            ..Default::default()
        };

        let handle = tokio::spawn(async move {
            let _ = AgentLoop::run(cfg, &provider, tx, None).await;
        });

        let mut tool_results = 0usize;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            tokio::select! {
                Some(msg) = rx.recv() => {
                    if matches!(msg, ServerMessage::ToolResult { .. }) {
                        tool_results += 1;
                    }
                    if matches!(msg, ServerMessage::MissionComplete { .. }) {
                        break;
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    break;
                }
            }
        }
        handle.abort();
        mock.shutdown().await;

        assert!(
            tool_results >= 2,
            "expected at least 2 ToolResult events (one per parallel read_file), got {tool_results}"
        );
    }

    #[test]
    fn plan_mode_system_prompt_has_plan_mode_instructions() {
        let config = AgentConfig {
            mission_goal: "Fix the CI pipeline".to_string(),
            workspace: "/workspace".to_string(),
            plan_mode: true,
            ..Default::default()
        };
        let prompt = build_system_prompt(&config);
        assert!(prompt.contains("PLAN MODE"));
        assert!(prompt.contains("Do NOT call any tools"));
    }

    #[test]
    fn normal_mode_system_prompt_has_tool_instructions() {
        let config = AgentConfig {
            mission_goal: "Fix the CI pipeline".to_string(),
            workspace: "/workspace".to_string(),
            plan_mode: false,
            ..Default::default()
        };
        let prompt = build_system_prompt(&config);
        assert!(!prompt.contains("PLAN MODE"));
        assert!(prompt.contains("Call `done` only when"));
    }

    #[test]
    fn system_prompt_injects_memory_context() {
        let config = AgentConfig {
            mission_goal: "Fix CI".to_string(),
            workspace: "/workspace".to_string(),
            memory_context: Some("Use multi-stage Docker builds.".to_string()),
            ..Default::default()
        };
        let prompt = build_system_prompt(&config);
        assert!(prompt.contains("## Memory"));
        assert!(prompt.contains("Use multi-stage Docker builds."));
    }

    #[test]
    fn system_prompt_no_memory_section_when_empty() {
        let config = AgentConfig {
            mission_goal: "Fix CI".to_string(),
            workspace: "/workspace".to_string(),
            memory_context: None,
            ..Default::default()
        };
        let prompt = build_system_prompt(&config);
        assert!(!prompt.contains("## Memory"));
    }
}
