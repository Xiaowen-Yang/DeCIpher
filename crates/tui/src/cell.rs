//! Cell trait and typed history cells.
//!
//! Each cell represents one logical entry in the chat history.
//! Cells produce `Vec<Line<'static>>` for ratatui rendering and know
//! their desired height at a given terminal width.

use std::any::Any;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

// ── Theme constants ────────────────────────────────────────────────────────

/// Shiba Inu orange (matches decipher_markdown::SHIBA).
const SHIBA: Color = Color::Rgb(232, 163, 23);
const DIM: Style = Style::new().add_modifier(Modifier::DIM);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
const GREEN: Style = Style::new().fg(Color::Green);
const RED: Style = Style::new().fg(Color::Red);
const CYAN: Style = Style::new().fg(Color::Cyan);
const YELLOW: Style = Style::new().fg(Color::Yellow);

// ── Cell trait ─────────────────────────────────────────────────────────────

/// A single entry in the chat history.
///
/// Cells are created from server messages and stored in `ChatWidget`.
/// They produce styled lines for display (viewport rendering) and for
/// the transcript pager (Ctrl+T).
pub trait Cell: std::fmt::Debug + Send + 'static {
    /// Styled lines for rendering in the terminal at the given width.
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;

    /// How many terminal rows this cell needs at the given width.
    fn desired_height(&self, width: u16) -> u16 {
        self.display_lines(width).len() as u16
    }

    /// Lines for the transcript pager. Defaults to `display_lines`.
    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.display_lines(width)
    }

    /// Whether this cell continues a previous cell (suppress blank separator).
    fn is_continuation(&self) -> bool {
        false
    }

    /// If this cell has an animation, return the current tick so the pager
    /// can use it as a cache-invalidation key. `None` = static.
    fn transcript_animation_tick(&self) -> Option<u64> {
        None
    }

    /// Downcast to concrete type.
    fn as_any(&self) -> &dyn Any;

    /// Downcast to concrete type (mutable).
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

// ── UserCell ───────────────────────────────────────────────────────────────

/// User input text (and optional image references).
#[derive(Debug)]
pub struct UserCell {
    pub text: String,
    pub images: Vec<String>, // base64 image refs (for future use)
}

impl UserCell {
    pub fn new(text: String, images: Vec<String>) -> Self {
        Self { text, images }
    }
}

impl Cell for UserCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for line in self.text.lines() {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(line.to_string(), BOLD),
            ]));
        }
        if !self.images.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  [{} image(s) attached]", self.images.len()),
                DIM,
            )));
        }
        if lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines
    }
}

// ── MissionCell ────────────────────────────────────────────────────────────

/// Mission understood — shows the parsed goal and plan steps.
#[derive(Debug)]
pub struct MissionCell {
    pub understood: String,
    pub target: Option<String>,
    pub steps: Vec<String>,
}

impl MissionCell {
    pub fn new(understood: String, target: Option<String>, steps: Vec<String>) -> Self {
        Self { understood, target, steps }
    }
}

impl Cell for MissionCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Header
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("MISSION ", Style::default().fg(SHIBA).add_modifier(Modifier::BOLD)),
            Span::styled("─".repeat(40), DIM),
        ]));

        // Understood
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(self.understood.clone(), Style::default()),
        ]));

        // Target
        if let Some(ref target) = self.target {
            lines.push(Line::from(vec![
                Span::styled("  Target: ", DIM),
                Span::styled(target.clone(), CYAN),
            ]));
        }

        // Steps
        if !self.steps.is_empty() {
            lines.push(Line::from(""));
            for (i, step) in self.steps.iter().enumerate() {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}. ", i + 1), DIM),
                    Span::styled(step.clone(), Style::default()),
                ]));
            }
        }

        lines
    }
}

// ── ExecCell ───────────────────────────────────────────────────────────────

/// A single tool call within an ExecCell.
#[derive(Debug)]
pub struct ExecCall {
    pub tool: String,
    pub output: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub success: Option<bool>,
    pub call_id: Option<String>,
}

/// Tool execution cell — may coalesce multiple read-only calls.
#[derive(Debug)]
pub struct ExecCell {
    pub calls: Vec<ExecCall>,
}

impl ExecCell {
    pub fn new(tool: String, reasoning: String) -> Self {
        Self {
            calls: vec![ExecCall {
                tool,
                output: Some(reasoning),
                elapsed_ms: None,
                success: None,
                call_id: None,
            }],
        }
    }

    /// Mark a call as completed (by index or call_id).
    pub fn complete_call(
        &mut self,
        tool: &str,
        success: bool,
        summary: String,
        elapsed_ms: u64,
    ) {
        // Find the last call matching this tool that isn't yet completed.
        if let Some(call) = self.calls.iter_mut().rev().find(|c| c.tool == tool && c.success.is_none()) {
            call.success = Some(success);
            call.output = Some(summary);
            call.elapsed_ms = Some(elapsed_ms);
        }
    }

    /// Add another tool call (coalescing).
    pub fn add_call(&mut self, tool: String, reasoning: String) {
        self.calls.push(ExecCall {
            tool,
            output: Some(reasoning),
            elapsed_ms: None,
            success: None,
            call_id: None,
        });
    }
}

impl Cell for ExecCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for call in &self.calls {
            let icon = match call.success {
                Some(true) => Span::styled("  \u{2713} ", GREEN),  // checkmark
                Some(false) => Span::styled("  \u{2717} ", RED),   // x mark
                None => Span::styled("  \u{2022} ", YELLOW),       // bullet (in progress)
            };

            let tool_span = Span::styled(call.tool.clone(), BOLD);

            let detail = if let Some(ref output) = call.output {
                let elapsed = call.elapsed_ms
                    .map(|ms| format!(" ({:.1}s)", ms as f64 / 1000.0))
                    .unwrap_or_default();
                vec![
                    icon,
                    tool_span,
                    Span::styled(format!(" \u{2014} {}{}", output, elapsed), DIM),
                ]
            } else {
                vec![icon, tool_span]
            };

            lines.push(Line::from(detail));
        }
        lines
    }
}

// ── AgentMessageCell ───────────────────────────────────────────────────────

/// Agent markdown response — pre-rendered lines.
#[derive(Debug)]
pub struct AgentMessageCell {
    /// Pre-rendered lines from the markdown stream collector.
    pub rendered_lines: Vec<Line<'static>>,
    /// Whether this is the first message in a sequence (adds top padding).
    pub is_first_line: bool,
}

impl AgentMessageCell {
    pub fn new(rendered_lines: Vec<Line<'static>>, is_first_line: bool) -> Self {
        Self { rendered_lines, is_first_line }
    }

    /// Create from raw markdown text (for non-streamed messages).
    pub fn from_text(text: &str) -> Self {
        // Simple fallback: plain text with 2-space indent.
        // In Phase 3 this will use the markdown renderer to produce styled lines.
        let rendered_lines = text
            .lines()
            .map(|line| Line::from(format!("  {}", line)))
            .collect();
        Self {
            rendered_lines,
            is_first_line: true,
        }
    }

    /// Append additional rendered lines (from streaming).
    pub fn append_lines(&mut self, lines: Vec<Line<'static>>) {
        self.rendered_lines.extend(lines);
    }
}

impl Cell for AgentMessageCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.rendered_lines.clone()
    }

    fn is_continuation(&self) -> bool {
        !self.is_first_line
    }
}

// ── ErrorCell ──────────────────────────────────────────────────────────────

/// Error message from the agent.
#[derive(Debug)]
pub struct ErrorCell {
    pub message: String,
}

impl ErrorCell {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl Cell for ErrorCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("ERROR ", RED.add_modifier(Modifier::BOLD)),
            Span::styled(self.message.clone(), RED),
        ]));
        lines
    }
}

// ── ResultCell ─────────────────────────────────────────────────────────────

/// Mission complete result.
#[derive(Debug)]
pub struct ResultCell {
    pub outcome: String,
    pub summary: String,
    pub turns: u32,
    pub elapsed_ms: u64,
}

impl ResultCell {
    pub fn new(outcome: String, summary: String, turns: u32, elapsed_ms: u64) -> Self {
        Self { outcome, summary, turns, elapsed_ms }
    }
}

impl Cell for ResultCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let secs = self.elapsed_ms as f64 / 1000.0;
        let mut lines = Vec::new();

        // Header
        let outcome_style = if self.outcome == "success" {
            GREEN.add_modifier(Modifier::BOLD)
        } else {
            RED.add_modifier(Modifier::BOLD)
        };
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(self.outcome.clone(), outcome_style),
            Span::styled(format!(" ({:.1}s, {} turns)", secs, self.turns), DIM),
        ]));

        // Summary
        for line in self.summary.lines() {
            lines.push(Line::from(format!("  {}", line)));
        }

        lines
    }
}

// ── ClarificationCell ─────────────────────────────────────────────────────

/// Agent is asking for clarification.
#[derive(Debug)]
pub struct ClarificationCell {
    pub question: String,
}

impl ClarificationCell {
    pub fn new(question: String) -> Self {
        Self { question }
    }
}

impl Cell for ClarificationCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("? ", YELLOW.add_modifier(Modifier::BOLD)),
            Span::styled(self.question.clone(), Style::default()),
        ]));
        lines
    }
}

// ── ApprovalCell ───────────────────────────────────────────────────────────

/// Agent needs user approval for an action.
#[derive(Debug)]
pub struct ApprovalCell {
    pub action: Option<String>,
    pub capabilities: Vec<String>,
    pub decision: Option<bool>,
}

impl ApprovalCell {
    pub fn new(action: Option<String>, capabilities: Vec<String>) -> Self {
        Self { action, capabilities, decision: None }
    }

    pub fn set_decision(&mut self, approved: bool) {
        self.decision = Some(approved);
    }
}

impl Cell for ApprovalCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if let Some(ref action) = self.action {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled("ACTION ", YELLOW.add_modifier(Modifier::BOLD)),
                Span::styled(action.clone(), Style::default()),
            ]));
        }

        for cap in &self.capabilities {
            lines.push(Line::from(vec![
                Span::styled("    \u{2022} ", DIM),
                Span::styled(cap.clone(), Style::default()),
            ]));
        }

        match self.decision {
            Some(true) => {
                lines.push(Line::from(Span::styled("  Approved", GREEN)));
            }
            Some(false) => {
                lines.push(Line::from(Span::styled("  Denied", RED)));
            }
            None => {
                lines.push(Line::from(Span::styled(
                    "  Waiting for approval... [y/a/n]",
                    DIM,
                )));
            }
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_cell_basic() {
        let cell = UserCell::new("hello world".into(), vec![]);
        let lines = cell.display_lines(80);
        assert_eq!(lines.len(), 1);
        assert_eq!(cell.desired_height(80), 1);
    }

    #[test]
    fn user_cell_multiline() {
        let cell = UserCell::new("line one\nline two\nline three".into(), vec![]);
        assert_eq!(cell.desired_height(80), 3);
    }

    #[test]
    fn user_cell_with_images() {
        let cell = UserCell::new("hello".into(), vec!["img1".into(), "img2".into()]);
        assert_eq!(cell.desired_height(80), 2); // text + image count
    }

    #[test]
    fn mission_cell_with_steps() {
        let cell = MissionCell::new(
            "Fix the Docker build".into(),
            Some("/app/Dockerfile".into()),
            vec!["Read Dockerfile".into(), "Fix COPY path".into()],
        );
        let lines = cell.display_lines(80);
        assert!(lines.len() >= 5); // header + understood + target + blank + 2 steps
    }

    #[test]
    fn exec_cell_complete() {
        let mut cell = ExecCell::new("git".into(), "clone repo".into());
        cell.complete_call("git", true, "cloned".into(), 2100);
        let lines = cell.display_lines(80);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn exec_cell_coalesce() {
        let mut cell = ExecCell::new("read_file".into(), "package.json".into());
        cell.add_call("read_file".into(), "Dockerfile".into());
        assert_eq!(cell.calls.len(), 2);
        assert_eq!(cell.desired_height(80), 2);
    }

    #[test]
    fn error_cell() {
        let cell = ErrorCell::new("connection refused".into());
        assert_eq!(cell.desired_height(80), 1);
    }

    #[test]
    fn result_cell_success() {
        let cell = ResultCell::new("success".into(), "All tests pass".into(), 5, 12000);
        let lines = cell.display_lines(80);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn clarification_cell() {
        let cell = ClarificationCell::new("Which branch?".into());
        assert_eq!(cell.desired_height(80), 1);
    }

    #[test]
    fn approval_cell_pending() {
        let cell = ApprovalCell::new(
            Some("run tests".into()),
            vec!["exec_command".into()],
        );
        assert!(cell.desired_height(80) >= 3); // action + cap + waiting
        assert_eq!(cell.decision, None);
    }

    #[test]
    fn approval_cell_approved() {
        let mut cell = ApprovalCell::new(None, vec![]);
        cell.set_decision(true);
        assert_eq!(cell.decision, Some(true));
    }

    #[test]
    fn agent_message_from_text() {
        let cell = AgentMessageCell::from_text("Hello\nWorld");
        assert_eq!(cell.desired_height(80), 2);
        assert!(cell.is_first_line);
        assert!(!cell.is_continuation());
    }

    #[test]
    fn agent_message_continuation() {
        let cell = AgentMessageCell::new(vec![], false);
        assert!(cell.is_continuation());
    }
}
