//! ChatWidget — cell lifecycle management.
//!
//! Manages committed (permanent) cells and one active (in-progress) cell.
//! Handles streaming deltas through MarkdownStreamCollector, flushing
//! active cells to committed, and creating typed cells from server messages.

use decipher_protocol::ServerMessage;

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
}

impl ChatWidget {
    pub fn new() -> Self {
        Self {
            committed_cells: Vec::new(),
            active_cell: None,
            active_cell_revision: 0,
            stream_collector: MarkdownStreamCollector::new(None),
            streaming: false,
        }
    }

    /// Set the terminal width for the markdown stream collector.
    pub fn set_width(&mut self, width: u16) {
        self.stream_collector.set_width(width);
    }

    /// Append a streaming delta to the active AgentMessageCell.
    ///
    /// If no active cell exists, creates one. Accumulates text in the
    /// MarkdownStreamCollector and commits complete lines.
    ///
    /// Returns the number of newly committed lines (for insert_before).
    pub fn push_delta(&mut self, delta: &str) -> Vec<ratatui::text::Line<'static>> {
        if !self.streaming {
            self.streaming = true;
            // Start a new AgentMessageCell as the active cell
            let is_first = self.active_cell.is_none();
            self.flush_active_cell();
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

    /// Flush the active cell to committed (move to permanent scrollback).
    pub fn flush_active_cell(&mut self) {
        if let Some(cell) = self.active_cell.take() {
            self.committed_cells.push(cell);
            self.active_cell_revision += 1;
        }
    }

    /// End the streaming sequence. Flushes remaining partial content,
    /// then moves the active cell to committed.
    pub fn end_stream(&mut self) {
        if self.streaming {
            let remaining = self.stream_collector.finalize_and_drain();
            if !remaining.is_empty() {
                if let Some(ref mut cell) = self.active_cell {
                    if let Some(agent_cell) = cell.as_any_mut().downcast_mut::<AgentMessageCell>() {
                        agent_cell.append_lines(remaining);
                    }
                }
            }
            self.streaming = false;
            self.flush_active_cell();
        }
    }

    /// Mark the active cell as failed (e.g., interrupted by Ctrl+C).
    pub fn finalize_active_cell_as_failed(&mut self) {
        if self.streaming {
            let _ = self.stream_collector.finalize_and_drain();
            self.streaming = false;
        }
        // Move whatever we have to committed
        self.flush_active_cell();
    }

    /// Whether streaming is currently in progress.
    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    /// Create typed cells from a server message.
    ///
    /// Some messages create an active cell (ToolStart, streaming deltas).
    /// Others flush immediately to committed.
    pub fn handle_server_message(&mut self, msg: &ServerMessage) {
        match msg {
            ServerMessage::Banner { .. } => {
                // Banner is rendered separately, not as a cell
            }

            ServerMessage::Mission { understood, target, steps, .. } => {
                // End any active stream first
                if self.streaming { self.end_stream(); }
                self.flush_active_cell();
                let cell = MissionCell::new(
                    understood.clone(),
                    target.clone(),
                    steps.clone(),
                );
                self.committed_cells.push(Box::new(cell));
            }

            ServerMessage::Clarification { question } => {
                if self.streaming { self.end_stream(); }
                self.flush_active_cell();
                let cell = ClarificationCell::new(question.clone());
                self.committed_cells.push(Box::new(cell));
            }

            ServerMessage::ApprovalRequest { action, capabilities } => {
                if self.streaming { self.end_stream(); }
                self.flush_active_cell();
                let action_str = action.as_ref().map(|a| {
                    let mut s = a.tool.clone();
                    if let Some(ref r) = a.reasoning {
                        s.push_str(" \u{2014} ");
                        s.push_str(r);
                    }
                    s
                });
                let cell = ApprovalCell::new(action_str, capabilities.clone());
                self.active_cell = Some(Box::new(cell));
                self.active_cell_revision += 1;
            }

            ServerMessage::ToolStart { tool, reasoning } => {
                if self.streaming { self.end_stream(); }
                // Coalesce read-only exploring calls into the same ExecCell
                let is_exploring = matches!(tool.as_str(), "read_file" | "list_files" | "search");
                if is_exploring {
                    if let Some(ref mut cell) = self.active_cell {
                        if let Some(exec_cell) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                            exec_cell.add_call(tool.clone(), reasoning.clone());
                            self.active_cell_revision += 1;
                            return;
                        }
                    }
                }
                // Non-exploring or no active ExecCell: flush and create new
                self.flush_active_cell();
                let cell = ExecCell::new(tool.clone(), reasoning.clone());
                self.active_cell = Some(Box::new(cell));
                self.active_cell_revision += 1;
            }

            ServerMessage::ToolResult { tool, success, summary, elapsed_ms } => {
                if let Some(ref mut cell) = self.active_cell {
                    if let Some(exec_cell) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                        exec_cell.complete_call(tool, *success, summary.clone(), *elapsed_ms);
                        self.active_cell_revision += 1;
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
                    self.flush_active_cell();
                }
            }

            ServerMessage::AgentMessage { text } => {
                // If we just finished streaming, this is a duplicate — skip
                if self.streaming {
                    self.end_stream();
                } else {
                    self.flush_active_cell();
                    let cell = AgentMessageCell::from_text(text);
                    self.committed_cells.push(Box::new(cell));
                }
            }

            ServerMessage::AgentMessageDelta { delta } => {
                self.push_delta(delta);
            }

            ServerMessage::MissionComplete { outcome, summary, turns, elapsed_ms, .. } => {
                if self.streaming { self.end_stream(); }
                self.flush_active_cell();
                let cell = ResultCell::new(
                    outcome.clone(),
                    summary.clone(),
                    *turns,
                    *elapsed_ms,
                );
                self.committed_cells.push(Box::new(cell));
            }

            ServerMessage::Error { message } => {
                if self.streaming { self.end_stream(); }
                self.flush_active_cell();
                let cell = ErrorCell::new(message.clone());
                self.committed_cells.push(Box::new(cell));
            }

            // These don't create cells
            ServerMessage::Spinner { .. } => {}
            ServerMessage::CommandList { .. } => {}
            ServerMessage::TokenUsage { .. } => {}
        }
    }

    /// Get transcript lines from all cells (for pager overlay).
    pub fn transcript_lines(&self, width: u16) -> Vec<ratatui::text::Line<'static>> {
        let mut lines = Vec::new();
        for cell in &self.committed_cells {
            if !cell.is_continuation() && !lines.is_empty() {
                lines.push(ratatui::text::Line::from(""));
            }
            lines.extend(cell.transcript_lines(width));
        }
        if let Some(ref cell) = self.active_cell {
            if !cell.is_continuation() && !lines.is_empty() {
                lines.push(ratatui::text::Line::from(""));
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
        let mut widget = ChatWidget::new();
        let msg = ServerMessage::Mission {
            understood: "Fix Docker".into(),
            target: Some("/Dockerfile".into()),
            target_type: None,
            steps: vec!["Read file".into()],
        };
        widget.handle_server_message(&msg);
        assert_eq!(widget.committed_cells.len(), 1);
        assert!(widget.active_cell.is_none());
    }

    #[test]
    fn tool_start_creates_active_cell() {
        let mut widget = ChatWidget::new();
        let msg = ServerMessage::ToolStart {
            tool: "exec_command".into(),
            reasoning: "running tests".into(),
        };
        widget.handle_server_message(&msg);
        assert_eq!(widget.committed_cells.len(), 0);
        assert!(widget.active_cell.is_some());
    }

    #[test]
    fn tool_result_flushes_cell() {
        let mut widget = ChatWidget::new();
        widget.handle_server_message(&ServerMessage::ToolStart {
            tool: "git".into(),
            reasoning: "clone".into(),
        });
        widget.handle_server_message(&ServerMessage::ToolResult {
            tool: "git".into(),
            success: true,
            summary: "cloned".into(),
            elapsed_ms: 2000,
        });
        assert_eq!(widget.committed_cells.len(), 1);
        assert!(widget.active_cell.is_none());
    }

    #[test]
    fn error_flushes_and_commits() {
        let mut widget = ChatWidget::new();
        widget.handle_server_message(&ServerMessage::Error {
            message: "connection refused".into(),
        });
        assert_eq!(widget.committed_cells.len(), 1);
    }

    #[test]
    fn transcript_lines_from_cells() {
        let mut widget = ChatWidget::new();
        widget.handle_server_message(&ServerMessage::AgentMessage {
            text: "Hello world".into(),
        });
        let lines = widget.transcript_lines(80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn finalize_as_failed_clears_streaming() {
        let mut widget = ChatWidget::new();
        widget.finalize_active_cell_as_failed();
        assert!(!widget.is_streaming());
        assert!(widget.active_cell.is_none());
    }
}
