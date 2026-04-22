//! ChatWidget — cell lifecycle management.
//!
//! Manages committed (permanent) cells and one active (in-progress) cell.
//! Handles streaming deltas through MarkdownStreamCollector, flushing
//! active cells to committed, and creating typed cells from server messages.
//!
//! `handle_server_message()` is the canonical entry point: it creates typed
//! cells AND returns the lines that should be inserted into terminal scrollback.

use decipher_protocol::ServerMessage;
use ratatui::text::Line;

use crate::cell::*;
use crate::markdown_stream::MarkdownStreamCollector;

/// Central widget that manages the chat history as typed cells.
#[derive(Debug)]
pub struct ChatWidget {
    /// Cells that have been finalized — permanent scrollback.
    pub committed_cells: Vec<Box<dyn Cell>>,
    /// The currently active cell (streaming or awaiting completion).
    pub active_cell: Option<Box<dyn Cell>>,
    /// Revision counter — bumps on every mutation to the active cell.
    /// Used as a cache-invalidation key by the pager.
    pub active_cell_revision: u64,
    /// Markdown stream collector for agent message deltas.
    stream_collector: MarkdownStreamCollector,
    /// Whether we are currently receiving streaming deltas.
    streaming: bool,
    /// Terminal width for display_lines.
    width: u16,
    /// Set when a write/exec ToolCard was flushed since the last GroupDivider.
    /// Used to decide whether to insert a divider before the next TaskCard.
    had_write_exec_since_divider: bool,
    /// Set when the last non-read-only exec completion had output preview (multi-line body).
    /// Used to insert Rule 2 empty-line spacing between consecutive multi-line exec blocks.
    /// Reset when the next agent reasoning turn begins (AgentMessage delta).
    last_exec_had_preview: bool,
    /// Files modified by write_file / apply_patch during the current task group.
    /// Cleared when a DiffCard is emitted.
    files_modified_pending: Vec<String>,
}

impl ChatWidget {
    pub fn new(width: u16) -> Self {
        Self {
            committed_cells: Vec::new(),
            active_cell: None,
            active_cell_revision: 0,
            stream_collector: MarkdownStreamCollector::new(Some(width)),
            streaming: false,
            width,
            had_write_exec_since_divider: false,
            last_exec_had_preview: false,
            files_modified_pending: Vec::new(),
        }
    }

    /// Get the current terminal width.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Set the terminal width for the markdown stream collector.
    pub fn set_width(&mut self, width: u16) {
        self.width = width;
        self.stream_collector.set_width(width);
    }

    /// The current partial (uncommitted) line from the stream collector.
    /// Used for streaming preview in the bottom pane.
    pub fn partial_line(&self) -> &str {
        self.stream_collector.partial_line()
    }

    /// Whether streaming is currently in progress.
    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    /// Append a streaming delta to the active AgentMessageCell.
    ///
    /// If no active cell exists, creates one. Accumulates text in the
    /// MarkdownStreamCollector and commits complete lines.
    ///
    /// Returns the newly committed lines (for `insert_before`).
    /// On the first delta of a new stream, prepends any flush output (e.g. a
    /// TaskCard summary) that was pending from the previous active cell.
    pub fn push_delta(&mut self, delta: &str) -> Vec<Line<'static>> {
        let flush_lines = if !self.streaming {
            self.streaming = true;
            self.last_exec_had_preview = false; // Rule 2: reset between agent reasoning turns
            let is_first = self.active_cell.is_none();
            let flushed = self.flush_and_emit();
            self.active_cell = Some(Box::new(AgentMessageCell::new(String::new(), is_first)));
            flushed
        } else {
            Vec::new()
        };

        // Snapshot committed position before pushing new delta.
        let prev_len = self.stream_collector.committed_raw().len();
        self.stream_collector.push_delta(delta);
        let new_lines = self.stream_collector.commit_complete_lines();
        if !new_lines.is_empty() {
            // Extract newly committed raw text.
            let new_raw = self.stream_collector.committed_raw()[prev_len..].to_string();
            if let Some(ref mut cell) = self.active_cell {
                if let Some(agent_cell) = cell.as_any_mut().downcast_mut::<AgentMessageCell>() {
                    agent_cell.append_raw(&new_raw);
                }
            }
            self.active_cell_revision += 1;
        }

        let mut all = flush_lines;
        all.extend(new_lines);
        all
    }

    /// Flush the active cell to committed without conversion or scrollback.
    ///
    /// Used by `finalize_active_cell_as_failed` and stream finalization where
    /// we deliberately skip TaskCard conversion.
    fn flush_active_cell_internal(&mut self) {
        if let Some(cell) = self.active_cell.take() {
            self.committed_cells.push(cell);
            self.active_cell_revision += 1;
        }
    }

    /// Flush the active cell to committed, converting read-only ExecCells to
    /// TaskCards and inserting GroupDividers as needed.
    ///
    /// Returns scrollback lines for any newly committed cells (TaskCard summary,
    /// GroupDivider). For write/exec ExecCells the scrollback was already emitted
    /// in the ToolStart/ToolResult handler, so this returns nothing for them.
    fn flush_and_emit(&mut self) -> Vec<Line<'static>> {
        let Some(cell) = self.active_cell.take() else {
            return Vec::new();
        };
        self.active_cell_revision += 1;
        let w = self.width;

        // Determine if this is a completed read-only group eligible for TaskCard.
        let readonly_data: Option<TaskCard> = {
            if let Some(ec) = cell.as_any().downcast_ref::<ExecCell>() {
                let is_ro = !ec.calls.is_empty()
                    && ec.calls.iter().all(|c| is_read_only_tool(&c.tool))
                    && ec.calls.iter().all(|c| c.success.is_some());
                if is_ro { Some(TaskCard::from_exec_cell(ec)) } else { None }
            } else {
                None
            }
            // borrow of `cell` via `ec` ends here
        };

        if let Some(task_card) = readonly_data {
            let task_lines = task_card.display_lines(w);
            let mut all_lines: Vec<Line<'static>> = Vec::new();

            // Insert GroupDivider before a new read-only group when write/exec
            // operations have already been flushed since the last divider.
            if self.had_write_exec_since_divider && !self.committed_cells.is_empty() {
                let divider = GroupDivider;
                all_lines.extend(divider.display_lines(w));
                self.committed_cells.push(Box::new(divider));
                self.had_write_exec_since_divider = false;
            }

            drop(cell);
            all_lines.extend(task_lines);
            self.committed_cells.push(Box::new(task_card));
            all_lines
        } else {
            // Track whether this was a write/exec group for future divider logic.
            let has_write_exec = cell.as_any().downcast_ref::<ExecCell>()
                .map(|ec| ec.calls.iter().any(|c| !is_read_only_tool(&c.tool)))
                .unwrap_or(false);
            if has_write_exec {
                self.had_write_exec_since_divider = true;
            }
            self.committed_cells.push(cell);
            Vec::new()
        }
    }

    /// Flush the active cell to committed (move to permanent scrollback).
    pub fn flush_active_cell(&mut self) {
        self.flush_active_cell_internal();
    }

    /// End the streaming sequence and return any remaining partial lines.
    /// The active cell is flushed to committed.
    fn end_stream_returning_remaining(&mut self) -> Vec<Line<'static>> {
        if !self.streaming {
            return Vec::new();
        }
        let partial_raw = self.stream_collector.partial_line().to_string();
        let remaining = self.stream_collector.finalize_and_drain();
        if !remaining.is_empty() {
            if let Some(ref mut cell) = self.active_cell {
                if let Some(agent_cell) = cell.as_any_mut().downcast_mut::<AgentMessageCell>() {
                    agent_cell.append_raw(&partial_raw);
                }
            }
        }
        self.streaming = false;
        self.flush_active_cell_internal();
        remaining
    }

    /// End the streaming sequence (no remaining lines returned).
    pub fn end_stream(&mut self) {
        self.end_stream_returning_remaining();
    }

    /// Mark the active cell as failed (e.g., interrupted by Ctrl+C).
    pub fn finalize_active_cell_as_failed(&mut self) {
        if self.streaming {
            let _ = self.stream_collector.finalize_and_drain();
            self.streaming = false;
        }
        self.flush_active_cell_internal();
    }

    /// Set the decision on the active ApprovalCell so the card shows [*]/[x] when flushed.
    /// Called from `app.respond_approval()` after the user makes their choice.
    pub fn resolve_active_approval(&mut self, approved: bool) {
        if let Some(ref mut cell) = self.active_cell {
            if let Some(ac) = cell.as_any_mut().downcast_mut::<ApprovalCell>() {
                ac.set_decision(approved);
                self.active_cell_revision += 1;
            }
        }
    }

    /// Process a server message: create typed cells and return scrollback lines.
    ///
    /// This is the canonical entry point for all server messages. It:
    /// 1. Creates/updates typed cells (for pager transcript)
    /// 2. Returns styled lines for `terminal.insert_before()` (scrollback)
    ///
    /// Banner is NOT handled here — it's a one-time header rendered separately.
    pub fn handle_server_message(&mut self, msg: &ServerMessage) -> Vec<Line<'static>> {
        let w = self.width;

        match msg {
            ServerMessage::Banner { .. } => {
                // Banner is rendered separately via banner_lines(), not as a cell
                Vec::new()
            }

            ServerMessage::Mission { understood, target, steps, .. } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                lines.extend(self.flush_and_emit());
                // Green ✓ line to signal mission understanding completed
                lines.push(Line::from(vec![
                    ratatui::text::Span::raw("  "),
                    ratatui::text::Span::styled(
                        "\u{2713}",
                        ratatui::style::Style::default().fg(ratatui::style::Color::Green),
                    ),
                    ratatui::text::Span::styled(
                        " Mission understood",
                        ratatui::style::Style::default()
                            .fg(ratatui::style::Color::Green)
                            .add_modifier(ratatui::style::Modifier::DIM),
                    ),
                ]));
                let cell = MissionCell::new(understood.clone(), target.clone(), steps.clone());
                lines.extend(cell.display_lines(w));
                self.committed_cells.push(Box::new(cell));
                lines
            }

            ServerMessage::Clarification { question } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                lines.extend(self.flush_and_emit());
                let cell = ClarificationCell::new(question.clone());
                lines.extend(cell.display_lines(w));
                self.committed_cells.push(Box::new(cell));
                lines
            }

            ServerMessage::ApprovalRequest { action, capabilities } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                lines.extend(self.flush_and_emit());
                let action_str = action.as_ref().map(|a| {
                    let mut s = a.tool.clone();
                    if let Some(ref r) = a.reasoning {
                        s.push_str(" \u{2014} ");
                        s.push_str(r);
                    }
                    s
                });
                let cell = ApprovalCell::new(action_str, capabilities.clone());
                lines.extend(cell.display_lines(w));
                self.active_cell = Some(Box::new(cell));
                self.active_cell_revision += 1;
                lines
            }

            ServerMessage::ToolStart { tool, reasoning, args, call_id } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }

                // Coalesce read-only calls into the same ExecCell.
                // They produce NO scrollback start lines — only the final TaskCard
                // summary (emitted on flush) appears in committed history.
                let is_exploring = is_read_only_tool(tool.as_str());
                if is_exploring {
                    if let Some(ref mut cell) = self.active_cell {
                        if let Some(exec_cell) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                            exec_cell.add_call(tool.clone(), reasoning.clone(), args.clone(), call_id.clone());
                            self.active_cell_revision += 1;
                            return lines;
                        }
                    }
                    // No active ExecCell to coalesce into — flush pending cell and
                    // create a new silent read-only cell.
                    lines.extend(self.flush_and_emit());
                    let cell = ExecCell::new(tool.clone(), reasoning.clone(), args.clone(), call_id.clone());
                    self.active_cell = Some(Box::new(cell));
                    self.active_cell_revision += 1;
                    return lines;
                }

                // Non-read-only tool: flush pending cell (may emit TaskCard summary)
                // then create a new active cell. NO scrollback emitted here —
                // the in-progress state is shown by the activity bar (blinking ●).
                // The completed ✓ line is emitted only on ToolResult.
                lines.extend(self.flush_and_emit());
                let cell = ExecCell::new(tool.clone(), reasoning.clone(), args.clone(), call_id.clone());
                self.active_cell = Some(Box::new(cell));
                self.active_cell_revision += 1;

                // Track write/exec tools for pending files_modified.
                if matches!(tool.as_str(), "write_file" | "apply_patch") {
                    if let Some(path) = args.as_ref()
                        .and_then(|a| a.get("path").or_else(|| a.get("target_file")))
                        .and_then(|v| v.as_str())
                    {
                        self.files_modified_pending.push(path.to_string());
                    }
                }

                lines
            }

            ServerMessage::ToolResult { tool, success, summary, elapsed_ms, exit_code, output_preview, output_lines_total, call_id, parsed_output, .. } => {
                let mut lines = Vec::new();
                if let Some(ref mut cell) = self.active_cell {
                    if let Some(exec_cell) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                        exec_cell.complete_call(
                            tool, *success, summary.clone(), *elapsed_ms,
                            *exit_code, output_preview.clone(), *output_lines_total,
                            call_id.as_deref(),
                        );
                        exec_cell.streaming_output.clear();
                        self.active_cell_revision += 1;

                        // Emit scrollback only for NON-read-only tools.
                        // Read-only tools will emit a compact TaskCard summary when flushed.
                        if !is_read_only_tool(tool) {
                            // Try smart card rendering first (Phase C).
                            let smart_emitted = if let Some(ref json_str) = parsed_output {
                                if let Some(smart_lines) = render_smart_card_lines(json_str, *success, *elapsed_ms) {
                                    exec_cell.smart_summary = Some(smart_lines.clone());
                                    self.active_cell_revision += 1;
                                    lines.extend(smart_lines);
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            // Fall back to generic completion line when no smart card.
                            if !smart_emitted {
                                let completed_call = exec_cell.calls.iter().rev()
                                    .find(|c| c.tool == *tool && c.success.is_some());
                                if let Some(call) = completed_call {
                                    // Rule 2: empty line between consecutive multi-line exec blocks.
                                    let has_preview = output_preview.as_ref().map_or(false, |p: &String| !p.is_empty());
                                    if self.last_exec_had_preview && has_preview {
                                        lines.push(Line::from(""));
                                    }
                                    let display = format_tool_display(
                                        &call.tool,
                                        call.args.as_ref(),
                                        call.output.as_deref().unwrap_or(""),
                                    );
                                    let icon = if *success {
                                        ratatui::text::Span::styled("\u{2713}", ratatui::style::Style::default().fg(ratatui::style::Color::Green))
                                    } else {
                                        ratatui::text::Span::styled("\u{2717}", ratatui::style::Style::default().fg(ratatui::style::Color::Red))
                                    };
                                    let s = *elapsed_ms as f64 / 1000.0;
                                    let exit_info = if !success {
                                        exit_code.map(|c| format!(" [exit {c}]")).unwrap_or_default()
                                    } else {
                                        String::new()
                                    };
                                    lines.push(Line::from(vec![
                                        ratatui::text::Span::raw("  "),
                                        icon,
                                        ratatui::text::Span::raw(format!(" {display}{exit_info} ")),
                                        ratatui::text::Span::styled(
                                            format!("({s:.1}s)"),
                                            ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM),
                                        ),
                                    ]));
                                    // Show output preview body lines.
                                    if let Some(ref preview) = output_preview {
                                        let preview_lines: Vec<&str> = preview.lines().collect();
                                        let is_write = matches!(
                                            call.tool.as_str(),
                                            "write_file" | "apply_patch"
                                        );
                                        if !success {
                                            // Error output: last 5 lines in red.
                                            let show = preview_lines.len().min(5);
                                            let start = preview_lines.len().saturating_sub(show);
                                            for (i, line_text) in preview_lines[start..].iter().enumerate() {
                                                let is_last = i == show - 1;
                                                let pfx = if is_last { "\u{2514}" } else { "\u{2502}" };
                                                let truncated: String = line_text.chars().take(100).collect();
                                                lines.push(Line::from(vec![
                                                    ratatui::text::Span::raw("    "),
                                                    ratatui::text::Span::styled(
                                                        format!("{pfx} "),
                                                        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM),
                                                    ),
                                                    ratatui::text::Span::styled(
                                                        truncated,
                                                        ratatui::style::Style::default().fg(ratatui::style::Color::Red).add_modifier(ratatui::style::Modifier::DIM),
                                                    ),
                                                ]));
                                            }
                                        } else if is_write && !preview_lines.is_empty() {
                                            // Write/patch diff: show content lines with +/- coloring.
                                            let show = preview_lines.len().min(12);
                                            for (i, line_text) in preview_lines.iter().take(show).enumerate() {
                                                let is_last = i == show - 1 && show == preview_lines.len();
                                                let pfx = if is_last { "\u{2514}" } else { "\u{2502}" };
                                                let truncated: String = line_text.chars().take(100).collect();
                                                let style = if line_text.starts_with('+') {
                                                    ratatui::style::Style::default().fg(ratatui::style::Color::Green).add_modifier(ratatui::style::Modifier::DIM)
                                                } else if line_text.starts_with('-') {
                                                    ratatui::style::Style::default().fg(ratatui::style::Color::Red).add_modifier(ratatui::style::Modifier::DIM)
                                                } else {
                                                    ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM)
                                                };
                                                lines.push(Line::from(vec![
                                                    ratatui::text::Span::raw("    "),
                                                    ratatui::text::Span::styled(
                                                        format!("{pfx} "),
                                                        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM),
                                                    ),
                                                    ratatui::text::Span::styled(truncated, style),
                                                ]));
                                            }
                                            if preview_lines.len() > show {
                                                lines.push(Line::from(vec![
                                                    ratatui::text::Span::raw("    "),
                                                    ratatui::text::Span::styled(
                                                        format!("  ({} more lines)", preview_lines.len() - show),
                                                        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM),
                                                    ),
                                                ]));
                                            }
                                        } else if *success && !preview_lines.is_empty() {
                                            // Success output: show last 3 lines (dim).
                                            let show = preview_lines.len().min(3);
                                            let start = preview_lines.len().saturating_sub(show);
                                            for (i, line_text) in preview_lines[start..].iter().enumerate() {
                                                let is_last = i == show - 1;
                                                let pfx = if is_last { "\u{2514}" } else { "\u{2502}" };
                                                let truncated: String = line_text.chars().take(100).collect();
                                                lines.push(Line::from(vec![
                                                    ratatui::text::Span::raw("    "),
                                                    ratatui::text::Span::styled(
                                                        format!("{pfx} "),
                                                        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM),
                                                    ),
                                                    ratatui::text::Span::styled(
                                                        truncated,
                                                        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM),
                                                    ),
                                                ]));
                                            }
                                            if preview_lines.len() > show {
                                                lines.push(Line::from(vec![
                                                    ratatui::text::Span::raw("    "),
                                                    ratatui::text::Span::styled(
                                                        format!("  ({} lines total)", preview_lines.len()),
                                                        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM),
                                                    ),
                                                ]));
                                            }
                                        }
                                    }
                                    // Update Rule 2 spacing state for the next emission.
                                    self.last_exec_had_preview = output_preview.as_ref().map_or(false, |p: &String| !p.is_empty());
                                }
                            }
                        }
                    }
                }

                // Flush when all calls in this cell are complete.
                // flush_and_emit will convert a completed read-only group to TaskCard.
                let all_done = self.active_cell.as_ref().map_or(false, |c| {
                    if let Some(exec_cell) = c.as_any().downcast_ref::<ExecCell>() {
                        exec_cell.calls.iter().all(|call| call.success.is_some())
                    } else {
                        false
                    }
                });
                if all_done {
                    lines.extend(self.flush_and_emit());
                }
                lines
            }

            ServerMessage::AgentMessage { text } => {
                if self.streaming {
                    // End stream — content already rendered via push_delta.
                    // Only return the remaining partial line.
                    self.end_stream_returning_remaining()
                } else {
                    let mut lines = self.flush_and_emit();
                    let cell = AgentMessageCell::from_text(text);
                    lines.extend(cell.display_lines(w));
                    self.committed_cells.push(Box::new(cell));
                    lines
                }
            }

            ServerMessage::AgentMessageDelta { delta } => {
                // Sanitize ANSI/OSC before streaming into the transcript.
                let clean = crate::cell::sanitize_display_text(delta);
                self.push_delta(&clean)
            }

            ServerMessage::MissionComplete {
                outcome, summary, turns, elapsed_ms,
                files_modified, errors_encountered, next_steps, ..
            } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                lines.extend(self.flush_and_emit());

                // Emit DiffCard before ResultCard when files were modified.
                // Use files from the protocol event, falling back to pending list.
                let diff_files: Vec<String> = if !files_modified.is_empty() {
                    files_modified.clone()
                } else {
                    std::mem::take(&mut self.files_modified_pending)
                };
                self.files_modified_pending.clear();

                if !diff_files.is_empty() {
                    let diff_card = DiffCard::new(diff_files, Vec::new());
                    lines.extend(diff_card.display_lines(w));
                    self.committed_cells.push(Box::new(diff_card));
                }

                let cell = ResultCell::new(
                    outcome.clone(), summary.clone(), *turns, *elapsed_ms,
                    files_modified.clone(), errors_encountered.clone(), next_steps.clone(),
                );
                lines.extend(cell.display_lines(w));
                self.committed_cells.push(Box::new(cell));
                // Reset divider tracking after mission complete.
                self.had_write_exec_since_divider = false;
                lines
            }

            ServerMessage::Error { message } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                lines.extend(self.flush_and_emit());
                let cell = ErrorCell::new(message.clone());
                lines.extend(cell.display_lines(w));
                self.committed_cells.push(Box::new(cell));
                lines
            }

            // Native function calling — mirrors ToolStart/ToolResult but via the
            // tool_call / tool_call_result protocol path.
            ServerMessage::ToolCall { call_id, name, input } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                let args = serde_json::from_str::<serde_json::Value>(input).ok();
                let is_exploring = is_read_only_tool(name.as_str());
                if is_exploring {
                    if let Some(ref mut cell) = self.active_cell {
                        if let Some(exec_cell) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                            exec_cell.add_call(name.clone(), input.chars().take(80).collect(), args, Some(call_id.clone()));
                            self.active_cell_revision += 1;
                            return lines;
                        }
                    }
                    lines.extend(self.flush_and_emit());
                    let cell = ExecCell::new(name.clone(), input.chars().take(80).collect(), args, Some(call_id.clone()));
                    self.active_cell = Some(Box::new(cell));
                    self.active_cell_revision += 1;
                    return lines;
                }
                lines.extend(self.flush_and_emit());
                let cell = ExecCell::new(name.clone(), input.chars().take(80).collect(), args, Some(call_id.clone()));
                lines.extend(cell.display_lines(w));
                self.active_cell = Some(Box::new(cell));
                self.active_cell_revision += 1;
                lines
            }

            ServerMessage::ToolCallResult { call_id, name, output, success } => {
                let mut lines = Vec::new();
                if let Some(ref mut cell) = self.active_cell {
                    if let Some(exec_cell) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                        let summary: String = output.chars().take(100).collect();
                        exec_cell.complete_call(name, *success, summary.clone(), 0, None, None, None, Some(call_id.as_str()));
                        self.active_cell_revision += 1;

                        // Only emit scrollback for non-read-only tools.
                        if !is_read_only_tool(name) {
                            let completed_call = exec_cell.calls.iter().rev()
                                .find(|c| c.tool == *name && c.success.is_some());
                            if let Some(call) = completed_call {
                                let display = format_tool_display(
                                    &call.tool, call.args.as_ref(), call.output.as_deref().unwrap_or(""),
                                );
                                let icon = if *success {
                                    ratatui::text::Span::styled("\u{2713}", ratatui::style::Style::default().fg(ratatui::style::Color::Green))
                                } else {
                                    ratatui::text::Span::styled("\u{2717}", ratatui::style::Style::default().fg(ratatui::style::Color::Red))
                                };
                                lines.push(Line::from(vec![
                                    ratatui::text::Span::raw("  "),
                                    icon,
                                    ratatui::text::Span::raw(format!(" {display} ")),
                                ]));
                            }
                        }
                    }
                }
                let all_done = self.active_cell.as_ref().map_or(false, |c| {
                    if let Some(ec) = c.as_any().downcast_ref::<ExecCell>() {
                        ec.calls.iter().all(|call| call.success.is_some())
                    } else { false }
                });
                if all_done {
                    lines.extend(self.flush_and_emit());
                }
                lines
            }

            // Exec output delta — append to active ExecCell for streaming preview.
            // No scrollback emitted; output visible in final ToolResult summary.
            ServerMessage::ExecOutputDelta { delta } => {
                if let Some(ref mut cell) = self.active_cell {
                    if let Some(exec_cell) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                        exec_cell.append_output(delta);
                        self.active_cell_revision += 1;
                    }
                }
                Vec::new()
            }

            // FilesModified: build a DiffCard for the current task group.
            ServerMessage::FilesModified { files } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                lines.extend(self.flush_and_emit());

                let paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
                if !paths.is_empty() {
                    // Build preview lines from the first file's preview data.
                    let preview = files.iter()
                        .flat_map(|f| f.preview.iter().map(|p| {
                            let kind = if p.starts_with('+') {
                                crate::cell::DiffPreviewKind::Add
                            } else {
                                crate::cell::DiffPreviewKind::Remove
                            };
                            crate::cell::DiffPreviewLine {
                                kind,
                                text: p.trim_start_matches(['+', '-', ' ']).to_string(),
                            }
                        }))
                        .take(3)
                        .collect();

                    let diff_card = DiffCard::new(paths, preview);
                    lines.extend(diff_card.display_lines(w));
                    self.committed_cells.push(Box::new(diff_card));
                    self.files_modified_pending.clear();
                }
                lines
            }

            // Subagent lifecycle events — rendered as AgentMessageCells.
            ServerMessage::SubagentStart { task, depth } => {
                let mut lines = self.flush_and_emit();
                let label = format!("[{} Subagent@{depth}] {task}", '\u{2193}');
                let cell = AgentMessageCell::from_text(&label);
                lines.extend(cell.display_lines(w));
                self.committed_cells.push(Box::new(cell));
                lines
            }
            ServerMessage::SubagentComplete { task, outcome, summary, depth } => {
                let mut lines = self.flush_and_emit();
                let label = format!(
                    "[{} Subagent@{depth} {outcome}] {task}: {}",
                    '\u{2191}',
                    summary.chars().take(80).collect::<String>()
                );
                let cell = AgentMessageCell::from_text(&label);
                lines.extend(cell.display_lines(w));
                self.committed_cells.push(Box::new(cell));
                lines
            }

            // Non-visual messages — no cells, no scrollback
            ServerMessage::Spinner { .. } => Vec::new(),
            ServerMessage::CommandList { .. } => Vec::new(),
            ServerMessage::TokenUsage { .. } => Vec::new(),
            ServerMessage::AgentStatus { .. } => Vec::new(),
        }
    }

    /// Get transcript lines from all cells (for pager overlay).
    pub fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for cell in &self.committed_cells {
            if !cell.is_continuation() && !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.extend(cell.transcript_lines(width));
        }
        if let Some(ref cell) = self.active_cell {
            if !cell.is_continuation() && !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.extend(cell.transcript_lines(width));
        }
        lines
    }

    /// Rebuild all scrollback lines from committed cells at the given width.
    ///
    /// Used on terminal resize to produce width-correct scrollback history.
    pub fn rebuild_scrollback(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for cell in &self.committed_cells {
            lines.extend(cell.display_lines(width));
        }
        lines
    }

    /// Cache key for transcript (committed count + active revision).
    pub fn transcript_cache_key(&self) -> (usize, u64) {
        let tick = self.active_cell
            .as_ref()
            .and_then(|c| c.transcript_animation_tick())
            .unwrap_or(0);
        (self.committed_cells.len(), self.active_cell_revision + tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decipher_protocol::ServerMessage;

    #[test]
    fn mission_creates_committed_cell() {
        let mut widget = ChatWidget::new(80);
        let msg = ServerMessage::Mission {
            understood: "Fix Docker".into(),
            target: Some("/Dockerfile".into()),
            target_type: None,
            steps: vec!["Read file".into()],
        };
        let lines = widget.handle_server_message(&msg);
        assert!(!lines.is_empty());
        assert_eq!(widget.committed_cells.len(), 1);
        assert!(widget.active_cell.is_none());
    }

    #[test]
    fn tool_start_creates_active_cell() {
        let mut widget = ChatWidget::new(80);
        let msg = ServerMessage::ToolStart {
            tool: "exec_command".into(),
            reasoning: "running tests".into(),
            args: Some(serde_json::json!({"cmd": "npm test"})),
            call_id: None,
        };
        let lines = widget.handle_server_message(&msg);
        // Non-read tools no longer emit scrollback on ToolStart — the in-progress
        // state is shown by the activity bar. Scrollback appears on ToolResult.
        assert!(lines.is_empty());
        assert!(widget.active_cell.is_some());
    }

    #[test]
    fn tool_result_flushes_cell() {
        let mut widget = ChatWidget::new(80);
        widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "git".into(),
            reasoning: "clone".into(),
            args: None,
            call_id: None,
        });
        let lines = widget.handle_server_message(&ServerMessage::ToolResult {
            tool: "git".into(),
            success: true,
            summary: "cloned".into(),
            elapsed_ms: 2000,
            exit_code: None,
            output_preview: None,
            output_lines_total: None,
            call_id: None,
            llm_text: None,
            parsed_output: None,
        });
        assert!(!lines.is_empty());
        assert_eq!(widget.committed_cells.len(), 1);
        assert!(widget.active_cell.is_none());
    }

    #[test]
    fn error_flushes_and_commits() {
        let mut widget = ChatWidget::new(80);
        let lines = widget.handle_server_message(&ServerMessage::Error {
            message: "connection refused".into(),
        });
        assert!(!lines.is_empty());
        assert_eq!(widget.committed_cells.len(), 1);
    }

    #[test]
    fn transcript_lines_from_cells() {
        let mut widget = ChatWidget::new(80);
        widget.handle_server_message(&ServerMessage::AgentMessage {
            text: "Hello world".into(),
        });
        let lines = widget.transcript_lines(80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn finalize_as_failed_clears_streaming() {
        let mut widget = ChatWidget::new(80);
        widget.finalize_active_cell_as_failed();
        assert!(!widget.is_streaming());
        assert!(widget.active_cell.is_none());
    }

    #[test]
    fn streaming_then_agent_message_suppresses() {
        let mut widget = ChatWidget::new(80);
        // Stream some content
        let delta_lines = widget.push_delta("Hello\n");
        assert!(!delta_lines.is_empty());
        assert!(widget.is_streaming());

        // AgentMessage arrives after streaming — should end stream, not create new cell
        let agent_lines = widget.handle_server_message(&ServerMessage::AgentMessage {
            text: "Hello".into(),
        });
        assert!(!widget.is_streaming());
        // The streaming cell was flushed to committed
        assert_eq!(widget.committed_cells.len(), 1);
        // Only remaining partial lines returned (if any)
        // In this case, no partial line since we had a complete "Hello\n"
        assert!(agent_lines.is_empty());
    }

    #[test]
    fn exec_cell_coalescing() {
        let mut widget = ChatWidget::new(80);
        // First read_file call
        widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "read_file".into(),
            reasoning: "package.json".into(),
            args: Some(serde_json::json!({"path": "package.json"})),
            call_id: None,
        });
        assert!(widget.active_cell.is_some());

        // Second read_file call — should coalesce
        widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "read_file".into(),
            reasoning: "Dockerfile".into(),
            args: Some(serde_json::json!({"path": "Dockerfile"})),
            call_id: None,
        });
        // Still one active cell with 2 calls
        let exec = widget.active_cell.as_ref().unwrap()
            .as_any().downcast_ref::<ExecCell>().unwrap();
        assert_eq!(exec.calls.len(), 2);
    }

    // ── Regression: protocol messages must not leak into scrollback ──────

    #[test]
    fn exec_output_delta_produces_no_scrollback() {
        let mut widget = ChatWidget::new(80);
        // Start a tool first (ExecOutputDelta only appends to active ExecCell)
        widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "exec_command".into(),
            reasoning: "clone repo".into(),
            args: Some(serde_json::json!({"cmd": "git clone ..."})),
            call_id: None,
        });
        // ExecOutputDelta must return empty — no scrollback lines
        let lines = widget.handle_server_message(&ServerMessage::ExecOutputDelta {
            delta: "Cloning into 'repo'...\n".into(),
        });
        assert!(lines.is_empty(), "ExecOutputDelta must not produce scrollback lines");
    }

    #[test]
    fn agent_status_produces_no_scrollback() {
        let mut widget = ChatWidget::new(80);
        let lines = widget.handle_server_message(&ServerMessage::AgentStatus {
            phase: "thinking".into(),
            turn: 2,
            max_turns: 20,
            elapsed_ms: 33170,
            tool_name: None,
        });
        assert!(lines.is_empty(), "AgentStatus must not produce scrollback lines");
    }

    #[test]
    fn spinner_produces_no_scrollback() {
        let mut widget = ChatWidget::new(80);
        let lines = widget.handle_server_message(&ServerMessage::Spinner {
            label: "Understanding mission".into(),
            done: false,
        });
        assert!(lines.is_empty(), "Spinner must not produce scrollback lines");
    }

    #[test]
    fn token_usage_produces_no_scrollback() {
        let mut widget = ChatWidget::new(80);
        let lines = widget.handle_server_message(&ServerMessage::TokenUsage {
            prompt_tokens: 1000,
            completion_tokens: 500,
            total_tokens: 1500,
            context_window: Some(128000),
        });
        assert!(lines.is_empty(), "TokenUsage must not produce scrollback lines");
    }

    // ── P1.0: Read-only ToolStart suppression ───────────────────────────

    #[test]
    fn read_file_tool_start_produces_no_scrollback() {
        let mut widget = ChatWidget::new(80);
        let lines = widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "read_file".into(),
            reasoning: "package.json".into(),
            args: Some(serde_json::json!({"path": "package.json"})),
            call_id: None,
        });
        assert!(lines.is_empty(), "read_file ToolStart should not emit scrollback lines");
        // But it should create an active cell for viewport display
        assert!(widget.active_cell.is_some());
    }

    #[test]
    fn list_files_tool_start_produces_no_scrollback() {
        let mut widget = ChatWidget::new(80);
        let lines = widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "list_files".into(),
            reasoning: "src/".into(),
            args: Some(serde_json::json!({"path": "src/"})),
            call_id: None,
        });
        assert!(lines.is_empty(), "list_files ToolStart should not emit scrollback lines");
    }

    #[test]
    fn exec_command_tool_start_no_scrollback() {
        let mut widget = ChatWidget::new(80);
        let lines = widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "exec_command".into(),
            reasoning: "build".into(),
            args: Some(serde_json::json!({"cmd": "npm test"})),
            call_id: None,
        });
        // Non-read tools no longer emit on ToolStart — only on ToolResult.
        assert!(lines.is_empty(), "exec_command ToolStart should NOT emit scrollback");
    }

    #[test]
    fn read_file_coalesce_produces_no_scrollback() {
        let mut widget = ChatWidget::new(80);
        // First read — creates silent active cell
        widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "read_file".into(),
            reasoning: "a.rs".into(),
            args: Some(serde_json::json!({"path": "a.rs"})),
            call_id: None,
        });
        // Second read — coalesces silently
        let lines = widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "read_file".into(),
            reasoning: "b.rs".into(),
            args: Some(serde_json::json!({"path": "b.rs"})),
            call_id: None,
        });
        assert!(lines.is_empty(), "coalesced read_file should not emit scrollback");
        // Active cell should have 2 calls
        let exec = widget.active_cell.as_ref().unwrap()
            .as_any().downcast_ref::<ExecCell>().unwrap();
        assert_eq!(exec.calls.len(), 2);
    }

    #[test]
    fn read_file_result_produces_scrollback() {
        let mut widget = ChatWidget::new(80);
        widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "read_file".into(),
            reasoning: "main.rs".into(),
            args: Some(serde_json::json!({"path": "src/main.rs"})),
            call_id: None,
        });
        let lines = widget.handle_server_message(&ServerMessage::ToolResult {
            tool: "read_file".into(),
            success: true,
            summary: "42 lines".into(),
            elapsed_ms: 50,
            exit_code: None,
            output_preview: None,
            output_lines_total: None,
            call_id: None,
            llm_text: None,
            parsed_output: None,
        });
        assert!(!lines.is_empty(), "read_file ToolResult should emit the compact result line");
    }

    // ── P1.0: Content sanitization in scrollback ──────────────────────

    #[test]
    fn agent_message_delta_strips_ansi() {
        let mut widget = ChatWidget::new(80);
        // Go through handle_server_message to test the full sanitization path.
        let lines = widget.handle_server_message(&ServerMessage::AgentMessageDelta {
            delta: "\x1b[31mred text\x1b[0m\n".into(),
        });
        // The scrollback line should not contain ANSI escapes
        for line in &lines {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(!text.contains('\x1b'), "ANSI leaked into delta scrollback: {text}");
        }
    }

    #[test]
    fn mission_complete_renders_clean_result() {
        let mut widget = ChatWidget::new(80);
        let lines = widget.handle_server_message(&ServerMessage::MissionComplete {
            outcome: "PASS".into(),
            summary: "Successfully cloned the repository https://github.com/example/repo".into(),
            turns: 3,
            elapsed_ms: 44800,
            urls: vec!["https://github.com/example/repo".into()],
            files_modified: vec![],
            errors_encountered: vec![],
            next_steps: vec![],
        });
        // Should produce display lines (result cell)
        assert!(!lines.is_empty());
        // None of the lines should contain raw JSON protocol markers
        for line in &lines {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(!text.contains(r#""type":"#), "Raw JSON leaked into result: {text}");
            assert!(!text.contains("]8;;"), "OSC hyperlink leaked into result: {text}");
            assert!(!text.starts_with("Error:"), "Error prefix in successful result: {text}");
        }
        // Should contain the outcome
        let all_text: String = lines.iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect::<Vec<_>>().join("");
        assert!(all_text.contains("PASS"), "Result should contain PASS");
        assert!(all_text.contains("RESULT:"), "Result should contain RESULT: label");
    }

    #[test]
    fn files_modified_creates_diff_card() {
        use decipher_protocol::FileModification;
        let mut widget = ChatWidget::new(80);
        let lines = widget.handle_server_message(&ServerMessage::FilesModified {
            files: vec![
                FileModification {
                    path: "src/main.rs".into(),
                    added: Some(5),
                    removed: Some(2),
                    preview: vec![
                        "+ fn new_function() {}".into(),
                        "- fn old_function() {}".into(),
                    ],
                },
                FileModification {
                    path: "src/lib.rs".into(),
                    added: Some(1),
                    removed: None,
                    preview: vec![],
                },
            ],
        });
        assert!(!lines.is_empty(), "FilesModified should emit scrollback lines");

        // Committed cells should contain a DiffCard
        let has_diff = widget.committed_cells.iter().any(|c| {
            c.as_any().downcast_ref::<crate::cell::DiffCard>().is_some()
        });
        assert!(has_diff, "FilesModified should push a DiffCard to committed_cells");

        let all_text: String = lines.iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect::<Vec<_>>().join("");
        assert!(all_text.contains("2 files edited"), "DiffCard header should say '2 files edited'");
    }

    #[test]
    fn mission_complete_with_files_emits_diff_card() {
        let mut widget = ChatWidget::new(80);
        widget.handle_server_message(&ServerMessage::MissionComplete {
            outcome: "PASS".into(),
            summary: "Done".into(),
            turns: 2,
            elapsed_ms: 10000,
            urls: vec![],
            files_modified: vec!["src/main.rs".into()],
            errors_encountered: vec![],
            next_steps: vec![],
        });

        // Both DiffCard and ResultCell should be committed
        let has_diff = widget.committed_cells.iter().any(|c| {
            c.as_any().downcast_ref::<crate::cell::DiffCard>().is_some()
        });
        let has_result = widget.committed_cells.iter().any(|c| {
            c.as_any().downcast_ref::<crate::cell::ResultCell>().is_some()
        });
        assert!(has_diff, "MissionComplete with files should commit a DiffCard");
        assert!(has_result, "MissionComplete should commit a ResultCell");

        // DiffCard must appear before ResultCell
        let diff_pos = widget.committed_cells.iter().position(|c| {
            c.as_any().downcast_ref::<crate::cell::DiffCard>().is_some()
        }).unwrap();
        let result_pos = widget.committed_cells.iter().position(|c| {
            c.as_any().downcast_ref::<crate::cell::ResultCell>().is_some()
        }).unwrap();
        assert!(diff_pos < result_pos, "DiffCard must appear before ResultCell");
    }

    #[test]
    fn agent_message_reflows_on_width_change() {
        // Text is short enough to fit on one line at width 200 (including 2-char indent),
        // but long enough to wrap at width 30.
        let mut widget = ChatWidget::new(200);
        let long_text = "Agent message that should reflow when the terminal width changes\n";
        let lines_wide = widget.push_delta(long_text);
        assert_eq!(lines_wide.len(), 1, "should be 1 line at width 200");
        widget.end_stream();
        // Transcript at narrow width must wrap
        let narrow_lines = widget.transcript_lines(30);
        assert!(narrow_lines.len() > 1, "should wrap at width 30, got {} lines", narrow_lines.len());
    }

    #[test]
    fn rebuild_scrollback_adapts_to_width() {
        let mut widget = ChatWidget::new(120);
        widget.handle_server_message(&ServerMessage::Mission {
            understood: "Fix the Docker build pipeline".into(),
            target: Some("/Dockerfile".into()),
            target_type: None,
            steps: vec!["Read Dockerfile".into(), "Fix syntax error".into()],
        });
        widget.handle_server_message(&ServerMessage::AgentMessage {
            text: "I have analyzed the Dockerfile and found a syntax error on line 42 where the RUN command is missing a backslash continuation".into(),
        });

        let wide = widget.rebuild_scrollback(120);
        let narrow = widget.rebuild_scrollback(40);
        assert!(narrow.len() >= wide.len(), "narrow ({}) should have >= lines than wide ({})", narrow.len(), wide.len());
    }
}
