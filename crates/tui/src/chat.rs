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
    pub fn push_delta(&mut self, delta: &str) -> Vec<Line<'static>> {
        if !self.streaming {
            self.streaming = true;
            let is_first = self.active_cell.is_none();
            self.flush_active_cell_internal();
            self.active_cell = Some(Box::new(AgentMessageCell::new(vec![], is_first)));
        }

        self.stream_collector.push_delta(delta);
        let new_lines = self.stream_collector.commit_complete_lines();
        if !new_lines.is_empty() {
            if let Some(ref mut cell) = self.active_cell {
                if let Some(agent_cell) = cell.as_any_mut().downcast_mut::<AgentMessageCell>() {
                    agent_cell.append_lines(new_lines.clone());
                }
            }
            self.active_cell_revision += 1;
        }
        new_lines
    }

    /// Flush the active cell to committed (internal — no scrollback lines returned).
    fn flush_active_cell_internal(&mut self) {
        if let Some(cell) = self.active_cell.take() {
            self.committed_cells.push(cell);
            self.active_cell_revision += 1;
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
        let remaining = self.stream_collector.finalize_and_drain();
        if !remaining.is_empty() {
            if let Some(ref mut cell) = self.active_cell {
                if let Some(agent_cell) = cell.as_any_mut().downcast_mut::<AgentMessageCell>() {
                    agent_cell.append_lines(remaining.clone());
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
                self.flush_active_cell_internal();
                let cell = MissionCell::new(understood.clone(), target.clone(), steps.clone());
                lines.extend(cell.display_lines(w));
                self.committed_cells.push(Box::new(cell));
                lines
            }

            ServerMessage::Clarification { question } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                self.flush_active_cell_internal();
                let cell = ClarificationCell::new(question.clone());
                lines.extend(cell.display_lines(w));
                self.committed_cells.push(Box::new(cell));
                lines
            }

            ServerMessage::ApprovalRequest { action, capabilities } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                self.flush_active_cell_internal();
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

            ServerMessage::ToolStart { tool, reasoning } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }

                // Coalesce read-only exploring calls into the same ExecCell
                let is_exploring = matches!(tool.as_str(), "read_file" | "list_files" | "search");
                if is_exploring {
                    if let Some(ref mut cell) = self.active_cell {
                        if let Some(exec_cell) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                            exec_cell.add_call(tool.clone(), reasoning.clone());
                            self.active_cell_revision += 1;
                            // Render the new call line for scrollback
                            let new_call = exec_cell.calls.last().unwrap();
                            let r: String = new_call.output.as_deref().unwrap_or("").chars().take(60).collect();
                            lines.push(Line::from(vec![
                                ratatui::text::Span::raw("  "),
                                ratatui::text::Span::styled("\u{2847}", ratatui::style::Style::default().fg(ratatui::style::Color::Cyan)),
                                ratatui::text::Span::raw(" "),
                                ratatui::text::Span::styled(
                                    format!("{} \u{2014} {}", new_call.tool, r),
                                    ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM),
                                ),
                            ]));
                            return lines;
                        }
                    }
                }

                // Non-exploring or no active ExecCell: flush and create new
                self.flush_active_cell_internal();
                let cell = ExecCell::new(tool.clone(), reasoning.clone());
                lines.extend(cell.display_lines(w));
                self.active_cell = Some(Box::new(cell));
                self.active_cell_revision += 1;
                lines
            }

            ServerMessage::ToolResult { tool, success, summary, elapsed_ms } => {
                let mut lines = Vec::new();
                if let Some(ref mut cell) = self.active_cell {
                    if let Some(exec_cell) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                        exec_cell.complete_call(tool, *success, summary.clone(), *elapsed_ms);
                        self.active_cell_revision += 1;

                        // Render the completed call for scrollback
                        let icon = if *success {
                            ratatui::text::Span::styled("\u{2713}", ratatui::style::Style::default().fg(ratatui::style::Color::Green))
                        } else {
                            ratatui::text::Span::styled("\u{2717}", ratatui::style::Style::default().fg(ratatui::style::Color::Red))
                        };
                        let s = *elapsed_ms as f64 / 1000.0;
                        lines.push(Line::from(vec![
                            ratatui::text::Span::raw("  "),
                            icon,
                            ratatui::text::Span::raw(format!(" {} \u{2014} {} ", tool, summary)),
                            ratatui::text::Span::styled(
                                format!("({s:.1}s)"),
                                ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM),
                            ),
                        ]));
                    }
                }

                // Flush if all calls are complete
                let all_done = self.active_cell.as_ref().map_or(false, |c| {
                    if let Some(exec_cell) = c.as_any().downcast_ref::<ExecCell>() {
                        exec_cell.calls.iter().all(|call| call.success.is_some())
                    } else {
                        false
                    }
                });
                if all_done {
                    self.flush_active_cell_internal();
                }
                lines
            }

            ServerMessage::AgentMessage { text } => {
                if self.streaming {
                    // End stream — content already rendered via push_delta.
                    // Only return the remaining partial line.
                    self.end_stream_returning_remaining()
                } else {
                    self.flush_active_cell_internal();
                    let cell = AgentMessageCell::from_text(text);
                    let lines = cell.display_lines(w);
                    self.committed_cells.push(Box::new(cell));
                    lines
                }
            }

            ServerMessage::AgentMessageDelta { delta } => {
                self.push_delta(delta)
            }

            ServerMessage::MissionComplete { outcome, summary, turns, elapsed_ms, .. } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                self.flush_active_cell_internal();
                let cell = ResultCell::new(outcome.clone(), summary.clone(), *turns, *elapsed_ms);
                lines.extend(cell.display_lines(w));
                self.committed_cells.push(Box::new(cell));
                lines
            }

            ServerMessage::Error { message } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                self.flush_active_cell_internal();
                let cell = ErrorCell::new(message.clone());
                lines.extend(cell.display_lines(w));
                self.committed_cells.push(Box::new(cell));
                lines
            }

            // Native function calling — display like ToolStart/ToolResult
            ServerMessage::ToolCall { name, input, .. } => {
                let mut lines = Vec::new();
                if self.streaming { lines.extend(self.end_stream_returning_remaining()); }
                // Coalesce into existing ExecCell if possible, else create new
                let is_exploring = matches!(name.as_str(), "read_file" | "list_files" | "search");
                if is_exploring {
                    if let Some(ref mut cell) = self.active_cell {
                        if let Some(exec_cell) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                            exec_cell.add_call(name.clone(), input.chars().take(80).collect());
                            self.active_cell_revision += 1;
                            return lines;
                        }
                    }
                }
                self.flush_active_cell_internal();
                let cell = ExecCell::new(name.clone(), input.chars().take(80).collect());
                lines.extend(cell.display_lines(w));
                self.active_cell = Some(Box::new(cell));
                self.active_cell_revision += 1;
                lines
            }

            ServerMessage::ToolCallResult { name, output, success, .. } => {
                let mut lines = Vec::new();
                if let Some(ref mut cell) = self.active_cell {
                    if let Some(exec_cell) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                        let summary: String = output.chars().take(100).collect();
                        exec_cell.complete_call(name, *success, summary.clone(), 0);
                        self.active_cell_revision += 1;
                        let icon = if *success {
                            ratatui::text::Span::styled("\u{2713}", ratatui::style::Style::default().fg(ratatui::style::Color::Green))
                        } else {
                            ratatui::text::Span::styled("\u{2717}", ratatui::style::Style::default().fg(ratatui::style::Color::Red))
                        };
                        lines.push(Line::from(vec![
                            ratatui::text::Span::raw("  "),
                            icon,
                            ratatui::text::Span::raw(format!(" {} \u{2014} {}", name, summary)),
                        ]));
                    }
                }
                let all_done = self.active_cell.as_ref().map_or(false, |c| {
                    if let Some(ec) = c.as_any().downcast_ref::<ExecCell>() {
                        ec.calls.iter().all(|call| call.success.is_some())
                    } else { false }
                });
                if all_done { self.flush_active_cell_internal(); }
                lines
            }

            // Non-visual messages — no cells, no scrollback
            ServerMessage::Spinner { .. } => Vec::new(),
            ServerMessage::CommandList { .. } => Vec::new(),
            ServerMessage::TokenUsage { .. } => Vec::new(),
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
        };
        let lines = widget.handle_server_message(&msg);
        assert!(!lines.is_empty()); // renders tool start line
        assert!(widget.active_cell.is_some());
    }

    #[test]
    fn tool_result_flushes_cell() {
        let mut widget = ChatWidget::new(80);
        widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "git".into(),
            reasoning: "clone".into(),
        });
        let lines = widget.handle_server_message(&ServerMessage::ToolResult {
            tool: "git".into(),
            success: true,
            summary: "cloned".into(),
            elapsed_ms: 2000,
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
        });
        assert!(widget.active_cell.is_some());

        // Second read_file call — should coalesce
        widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "read_file".into(),
            reasoning: "Dockerfile".into(),
        });
        // Still one active cell with 2 calls
        let exec = widget.active_cell.as_ref().unwrap()
            .as_any().downcast_ref::<ExecCell>().unwrap();
        assert_eq!(exec.calls.len(), 2);
    }
}
