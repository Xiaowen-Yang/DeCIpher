//! Cell trait and typed history cells.
//!
//! Each cell represents one logical entry in the chat history.
//! Cells produce `Vec<Line<'static>>` for ratatui rendering and know
//! their desired height at a given terminal width.

use std::any::Any;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

// ── Theme constants ────────────────────────────────────────────────────────

const DIM: Style = Style::new().add_modifier(Modifier::DIM);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
const GREEN: Style = Style::new().fg(Color::Green);
const RED: Style = Style::new().fg(Color::Red);
const CYAN: Style = Style::new().fg(Color::Cyan);
const BOLD_CYAN: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const YELLOW: Style = Style::new().fg(Color::Rgb(232, 163, 23));
const BOLD_YELLOW: Style = Style::new().fg(Color::Rgb(232, 163, 23)).add_modifier(Modifier::BOLD);

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
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("\u{250c} ", DIM),
            Span::styled("MISSION", BOLD_CYAN),
            Span::raw(" "),
            Span::styled("\u{2500}".repeat(40), DIM),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Understood: ", BOLD_YELLOW),
            Span::styled(self.understood.clone(), Style::default().fg(Color::White)),
        ]));
        if let Some(ref target) = self.target {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("Target: ", DIM),
                Span::styled(target.clone(), CYAN),
            ]));
        }
        if !self.steps.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("Plan:", DIM),
            ]));
            for (i, step) in self.steps.iter().enumerate() {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(format!("{}. ", i + 1), DIM),
                    Span::raw(step.clone()),
                ]));
            }
        }
        lines.push(Line::from(""));
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
    /// Parsed args for display (e.g., command string, file path).
    pub args: Option<serde_json::Value>,
    /// Process exit code (exec_command only).
    pub exit_code: Option<i32>,
    /// First few lines of output for preview.
    pub output_preview: Option<String>,
    /// Total output lines.
    pub output_lines_total: Option<u32>,
}

/// Tool execution cell — may coalesce multiple read-only calls.
#[derive(Debug)]
pub struct ExecCell {
    pub calls: Vec<ExecCall>,
    /// Accumulated streaming output from exec_command (real-time).
    pub streaming_output: String,
}

/// Max lines of streaming output to show in the cell.
const EXEC_OUTPUT_PREVIEW_LINES: usize = 5;

impl ExecCell {
    pub fn new(tool: String, reasoning: String, args: Option<serde_json::Value>) -> Self {
        Self {
            calls: vec![ExecCall {
                tool,
                output: Some(reasoning),
                elapsed_ms: None,
                success: None,
                call_id: None,
                args,
                exit_code: None,
                output_preview: None,
                output_lines_total: None,
            }],
            streaming_output: String::new(),
        }
    }

    /// Append streaming output from exec_command.
    pub fn append_output(&mut self, delta: &str) {
        self.streaming_output.push_str(delta);
    }

    /// Mark a call as completed (by index or call_id).
    pub fn complete_call(
        &mut self,
        tool: &str,
        success: bool,
        summary: String,
        elapsed_ms: u64,
        exit_code: Option<i32>,
        output_preview: Option<String>,
        output_lines_total: Option<u32>,
    ) {
        // Find the last call matching this tool that isn't yet completed.
        if let Some(call) = self.calls.iter_mut().rev().find(|c| c.tool == tool && c.success.is_none()) {
            call.success = Some(success);
            call.output = Some(summary);
            call.elapsed_ms = Some(elapsed_ms);
            call.exit_code = exit_code;
            call.output_preview = output_preview;
            call.output_lines_total = output_lines_total;
        }
    }

    /// Add another tool call (coalescing).
    pub fn add_call(&mut self, tool: String, reasoning: String, args: Option<serde_json::Value>) {
        self.calls.push(ExecCall {
            tool,
            output: Some(reasoning),
            elapsed_ms: None,
            success: None,
            call_id: None,
            args,
            exit_code: None,
            output_preview: None,
            output_lines_total: None,
        });
    }
}

/// Format a tool call for display based on tool type.
/// Returns a display string like `$ docker build .` or `Read: src/main.rs`
pub fn format_tool_display(tool: &str, args: Option<&serde_json::Value>, reasoning: &str) -> String {
    match tool {
        "exec_command" => {
            if let Some(cmd) = args.and_then(|a| a.get("cmd")).and_then(|v| v.as_str()) {
                let truncated: String = cmd.chars().take(120).collect();
                format!("$ {truncated}")
            } else {
                let r: String = reasoning.chars().take(80).collect();
                format!("$ {r}")
            }
        }
        "read_file" => {
            if let Some(path) = args.and_then(|a| a.get("path")).and_then(|v| v.as_str()) {
                format!("Read: {path}")
            } else {
                let r: String = reasoning.chars().take(80).collect();
                format!("Read: {r}")
            }
        }
        "write_file" => {
            if let Some(path) = args.and_then(|a| a.get("path")).and_then(|v| v.as_str()) {
                format!("Write: {path}")
            } else {
                let r: String = reasoning.chars().take(80).collect();
                format!("Write: {r}")
            }
        }
        "apply_patch" => {
            if let Some(path) = args.and_then(|a| a.get("target_file")).and_then(|v| v.as_str()) {
                format!("Patch: {path}")
            } else {
                "Patch: (unified diff)".to_string()
            }
        }
        "update_plan" => "Update plan".to_string(),
        "done" => "Done".to_string(),
        _ => {
            // kubectl_get, kubectl_logs, etc. — show as $ command
            if let Some(cmd) = args.and_then(|a| a.get("command")).and_then(|v| v.as_str()) {
                format!("$ {cmd}")
            } else {
                let r: String = reasoning.chars().take(60).collect();
                format!("{tool} \u{2014} {r}")
            }
        }
    }
}

impl Cell for ExecCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for call in &self.calls {
            let display = format_tool_display(
                &call.tool,
                call.args.as_ref(),
                call.output.as_deref().unwrap_or(""),
            );
            // Read-only tools get dimmed styling for lower noise
            let is_read_only = matches!(
                call.tool.as_str(),
                "read_file" | "kubectl_get" | "kubectl_logs" | "kubectl_describe" | "kubectl_events"
            );

            match call.success {
                None => {
                    // In-progress: show spinner icon + tool display
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("\u{2847}", CYAN),
                        Span::raw(" "),
                        Span::styled(display, DIM),
                    ]));
                }
                Some(success) => {
                    let icon = if success {
                        Span::styled("\u{2713}", if is_read_only { DIM } else { GREEN })
                    } else {
                        Span::styled("\u{2717}", RED)
                    };
                    let elapsed = call.elapsed_ms
                        .map(|ms| format!(" ({:.1}s)", ms as f64 / 1000.0))
                        .unwrap_or_default();
                    let exit_info = if !success {
                        call.exit_code.map(|c| format!(" [exit {}]", c)).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let text_style = if is_read_only && success { DIM } else { Style::default() };
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        icon,
                        Span::styled(format!(" {display}{exit_info} "), text_style),
                        Span::styled(elapsed, DIM),
                    ]));

                    // Show output preview: error output for failures, elided summary for long successful output
                    if let Some(ref preview) = call.output_preview {
                        let preview_lines: Vec<&str> = preview.lines().collect();
                        let total_lines = call.output_lines_total.unwrap_or(preview_lines.len() as u32);
                        let style = if success {
                            DIM // dimmed for successful output
                        } else {
                            Style::new().fg(Color::Red).add_modifier(Modifier::DIM)
                        };

                        if !success {
                            // Error: show last few lines
                            let show = preview_lines.len().min(5);
                            let start = preview_lines.len().saturating_sub(show);
                            for (i, line_text) in preview_lines[start..].iter().enumerate() {
                                let is_last = i == show - 1;
                                let prefix = if is_last { "\u{2514}" } else { "\u{2502}" };
                                let truncated: String = line_text.chars().take(100).collect();
                                lines.push(Line::from(vec![
                                    Span::raw("    "),
                                    Span::styled(format!("{prefix} "), DIM),
                                    Span::styled(truncated, style),
                                ]));
                            }
                        } else if total_lines > 8 {
                            // Success with long output: show head(3) + ... + tail(2)
                            let head: Vec<&str> = preview_lines.iter().take(3).copied().collect();
                            let tail: Vec<&str> = preview_lines.iter().rev().take(2).rev().copied().collect();
                            for line_text in &head {
                                let truncated: String = line_text.chars().take(100).collect();
                                lines.push(Line::from(vec![
                                    Span::raw("    "),
                                    Span::styled("\u{2502} ", DIM),
                                    Span::styled(truncated, style),
                                ]));
                            }
                            let hidden = total_lines.saturating_sub(5);
                            lines.push(Line::from(vec![
                                Span::raw("    "),
                                Span::styled(format!("\u{2502} \u{2026} ({hidden} lines hidden)"), DIM),
                            ]));
                            for (i, line_text) in tail.iter().enumerate() {
                                let is_last = i == tail.len() - 1;
                                let prefix = if is_last { "\u{2514}" } else { "\u{2502}" };
                                let truncated: String = line_text.chars().take(100).collect();
                                lines.push(Line::from(vec![
                                    Span::raw("    "),
                                    Span::styled(format!("{prefix} "), DIM),
                                    Span::styled(truncated, style),
                                ]));
                            }
                        }

                        if total_lines > 8 || !success {
                            if total_lines > 5 && !success {
                                lines.push(Line::from(vec![
                                    Span::raw("    "),
                                    Span::styled(format!("  ({total_lines} lines total)"), DIM),
                                ]));
                            }
                        }
                    }
                }
            }
        }
        // Show streaming output preview (last N lines, dimmed, with tree prefix)
        if !self.streaming_output.is_empty() {
            let output_lines: Vec<&str> = self.streaming_output.lines().collect();
            let total = output_lines.len();
            let start = total.saturating_sub(EXEC_OUTPUT_PREVIEW_LINES);
            if start > 0 {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        format!("\u{2502} \u{2026} +{} lines", start),
                        DIM,
                    ),
                ]));
            }
            for (i, line_text) in output_lines[start..].iter().enumerate() {
                let is_last = i == output_lines[start..].len() - 1;
                let prefix = if is_last { "\u{2514} " } else { "\u{2502} " };
                let truncated: String = line_text.chars().take(100).collect();
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(prefix, DIM),
                    Span::styled(truncated, DIM),
                ]));
            }
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

/// Error message from the agent — structured with context.
#[derive(Debug)]
pub struct ErrorCell {
    pub message: String,
    /// Error category for display styling.
    pub category: ErrorCategory,
    /// Retry info (e.g., "2/3").
    pub retry_info: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ErrorCategory {
    /// API error (rate limit, auth, context overflow).
    Api,
    /// Tool parse failure (LLM returned invalid JSON).
    Parse,
    /// Command execution failure.
    Execution,
    /// Generic/unknown error.
    Generic,
}

impl ErrorCell {
    pub fn new(message: String) -> Self {
        // Auto-detect error category from message content
        let category = if message.contains("rate limit") || message.contains("429") || message.contains("API error") {
            ErrorCategory::Api
        } else if message.contains("parse") || message.contains("JSON") || message.contains("tool call") {
            ErrorCategory::Parse
        } else if message.contains("exit") || message.contains("command") || message.contains("FAILED") {
            ErrorCategory::Execution
        } else {
            ErrorCategory::Generic
        };
        Self { message, category, retry_info: None }
    }

    pub fn with_retry(mut self, retry: String) -> Self {
        self.retry_info = Some(retry);
        self
    }
}

impl Cell for ErrorCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Category label
        let cat_label = match self.category {
            ErrorCategory::Api => "API Error",
            ErrorCategory::Parse => "Parse Error",
            ErrorCategory::Execution => "Execution Error",
            ErrorCategory::Generic => "Error",
        };

        let mut header_spans = vec![
            Span::raw("  "),
            Span::styled(format!("{cat_label}: "), RED),
        ];

        // Retry info if available
        if let Some(ref retry) = self.retry_info {
            header_spans.push(Span::styled(format!("[retry {retry}] "), YELLOW));
        }

        // First line of error message on same line as label
        let msg_lines: Vec<&str> = self.message.lines().collect();
        if let Some(first) = msg_lines.first() {
            let truncated: String = first.chars().take(100).collect();
            header_spans.push(Span::styled(truncated, Style::new().fg(Color::Red).add_modifier(Modifier::DIM)));
        }
        lines.push(Line::from(header_spans));

        // Additional lines indented (up to 5 lines)
        for line_text in msg_lines.iter().skip(1).take(5) {
            let truncated: String = line_text.chars().take(100).collect();
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(truncated, Style::new().fg(Color::Red).add_modifier(Modifier::DIM)),
            ]));
        }
        if msg_lines.len() > 6 {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(format!("... ({} more lines)", msg_lines.len() - 6), DIM),
            ]));
        }

        lines
    }
}

// ── ResultCell ─────────────────────────────────────────────────────────────

/// Mission complete result — structured with files, errors, and next steps.
#[derive(Debug)]
pub struct ResultCell {
    pub outcome: String,
    pub summary: String,
    pub turns: u32,
    pub elapsed_ms: u64,
    pub files_modified: Vec<String>,
    pub errors_encountered: Vec<String>,
    pub next_steps: Vec<String>,
}

impl ResultCell {
    pub fn new(
        outcome: String, summary: String, turns: u32, elapsed_ms: u64,
        files_modified: Vec<String>, errors_encountered: Vec<String>, next_steps: Vec<String>,
    ) -> Self {
        Self { outcome, summary, turns, elapsed_ms, files_modified, errors_encountered, next_steps }
    }
}

/// Style for the PARTIAL outcome (yellow).
const PARTIAL_STYLE: Style = Style::new().fg(Color::Rgb(232, 163, 23));

impl Cell for ResultCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let secs = self.elapsed_ms as f64 / 1000.0;
        let w = 60;
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("\u{2500}".repeat(w), DIM)));
        lines.push(Line::from(""));

        // Outcome header with color
        let (outcome_span, header_style) = match self.outcome.as_str() {
            "PASS" => (Span::styled(format!("PASS ({secs:.1}s)"), GREEN), GREEN),
            "PARTIAL" => (Span::styled(format!("PARTIAL ({secs:.1}s)"), PARTIAL_STYLE), PARTIAL_STYLE),
            _ => (Span::styled(format!("FAIL ({secs:.1}s)"), RED), RED),
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("[RESULT]", BOLD),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  Outcome:     "),
            outcome_span,
        ]));
        lines.push(Line::from(format!("  Turns:       {}", self.turns)));

        // Summary
        if !self.summary.is_empty() {
            lines.push(Line::from(""));
            for line in self.summary.lines() {
                lines.push(Line::from(format!("  {line}")));
            }
        }

        // Files modified
        if !self.files_modified.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("Files modified:", DIM),
            ]));
            for f in &self.files_modified {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled("\u{2022} ", header_style),
                    Span::styled(f.clone(), CYAN),
                ]));
            }
        }

        // Errors encountered
        if !self.errors_encountered.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("Errors:", RED),
            ]));
            for e in &self.errors_encountered {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled("\u{2022} ", RED),
                    Span::styled(e.clone(), Style::new().fg(Color::Red).add_modifier(Modifier::DIM)),
                ]));
            }
        }

        // Next steps (for FAIL/PARTIAL)
        if !self.next_steps.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("Next steps:", BOLD),
            ]));
            for (i, step) in self.next_steps.iter().enumerate() {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(format!("{}. ", i + 1), DIM),
                    Span::raw(step.clone()),
                ]));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("\u{2500}".repeat(w), DIM)));
        lines.push(Line::from(""));
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
        vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("\u{250c} ", DIM),
                Span::styled("CLARIFICATION NEEDED", YELLOW),
                Span::raw(" "),
                Span::styled("\u{2500}".repeat(30), DIM),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled("DeCIpher asks: ", BOLD_YELLOW),
                Span::styled(self.question.clone(), Style::default().fg(Color::White)),
            ]),
            Line::from(Span::styled("  Reply below and DeCIpher will continue.", DIM)),
            Line::from(""),
        ]
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
        let bar = "\u{2500}".repeat(56);
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  \u{250c}\u{2500} ", DIM),
            Span::styled("APPROVAL", BOLD_YELLOW),
            Span::styled(format!(" {}", bar), DIM),
        ]));
        lines.push(Line::from(Span::styled("  \u{2502}", DIM)));
        if let Some(ref action) = self.action {
            lines.push(Line::from(vec![
                Span::styled("  \u{2502} ", DIM),
                Span::styled("Action: ", BOLD),
                Span::styled(action.clone(), CYAN),
            ]));
            lines.push(Line::from(Span::styled("  \u{2502}", DIM)));
        }
        lines.push(Line::from(vec![
            Span::styled("  \u{2502} ", DIM),
            Span::raw("DeCIpher requests these capabilities:"),
        ]));
        for cap in &self.capabilities {
            lines.push(Line::from(vec![
                Span::styled("  \u{2502}   ", DIM),
                Span::styled("\u{203a} ", YELLOW),
                Span::raw(cap.clone()),
            ]));
        }
        lines.push(Line::from(Span::styled("  \u{2502}", DIM)));
        match self.decision {
            Some(true) => {
                lines.push(Line::from(vec![
                    Span::styled("  \u{2502}  ", DIM),
                    Span::styled("Approved", GREEN),
                ]));
            }
            Some(false) => {
                lines.push(Line::from(vec![
                    Span::styled("  \u{2502}  ", DIM),
                    Span::styled("Denied", RED),
                ]));
            }
            None => {
                lines.push(Line::from(vec![
                    Span::styled("  \u{2502}  ", DIM),
                    Span::styled("y", BOLD),
                    Span::styled(" approve  ", DIM),
                    Span::styled("a", BOLD),
                    Span::styled(" always  ", DIM),
                    Span::styled("n", BOLD),
                    Span::styled(" deny", DIM),
                ]));
            }
        }
        lines.push(Line::from(Span::styled(format!("  \u{2514}{}\u{2500}", bar), DIM)));
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
        // empty + header + understood + target + blank + Plan: + 2 steps + empty = 9
        assert!(lines.len() >= 7);
    }

    #[test]
    fn exec_cell_complete() {
        let mut cell = ExecCell::new("git".into(), "clone repo".into(), None);
        cell.complete_call("git", true, "cloned".into(), 2100, None, None, None);
        let lines = cell.display_lines(80);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn exec_cell_coalesce() {
        let mut cell = ExecCell::new("read_file".into(), "package.json".into(), Some(serde_json::json!({"path": "package.json"})));
        cell.add_call("read_file".into(), "Dockerfile".into(), Some(serde_json::json!({"path": "Dockerfile"})));
        assert_eq!(cell.calls.len(), 2);
        assert_eq!(cell.desired_height(80), 2);
    }

    #[test]
    fn exec_cell_rich_display_command() {
        let cell = ExecCell::new(
            "exec_command".into(),
            "build the project".into(),
            Some(serde_json::json!({"cmd": "docker build ."})),
        );
        let lines = cell.display_lines(80);
        assert_eq!(lines.len(), 1);
        // The display should contain "$ docker build ."
        let line_str: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(line_str.contains("$ docker build ."), "Expected '$ docker build .' in '{line_str}'");
    }

    #[test]
    fn exec_cell_rich_display_read() {
        let cell = ExecCell::new(
            "read_file".into(),
            "reading config".into(),
            Some(serde_json::json!({"path": "src/main.rs"})),
        );
        let lines = cell.display_lines(80);
        let line_str: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(line_str.contains("Read: src/main.rs"), "Expected 'Read: src/main.rs' in '{line_str}'");
    }

    #[test]
    fn exec_cell_failed_with_preview() {
        let mut cell = ExecCell::new(
            "exec_command".into(),
            "run tests".into(),
            Some(serde_json::json!({"cmd": "npm test"})),
        );
        cell.complete_call(
            "exec_command", false, "tests failed".into(), 5000,
            Some(1), Some("FAIL src/app.test.js\nExpected 5 but got 3".into()), Some(42),
        );
        let lines = cell.display_lines(80);
        // Should have: result line + 2 preview lines
        assert!(lines.len() >= 3, "Expected >= 3 lines, got {}", lines.len());
    }

    #[test]
    fn error_cell() {
        let cell = ErrorCell::new("connection refused".into());
        assert_eq!(cell.desired_height(80), 1);
    }

    #[test]
    fn result_cell_success() {
        let cell = ResultCell::new("PASS".into(), "All tests pass".into(), 5, 12000, vec![], vec![], vec![]);
        let lines = cell.display_lines(80);
        assert!(lines.len() >= 6);
    }

    #[test]
    fn result_cell_fail_with_details() {
        let cell = ResultCell::new(
            "FAIL".into(),
            "Docker build failed".into(),
            8, 30000,
            vec!["Dockerfile".into(), "requirements.txt".into()],
            vec!["pip install failed (exit 1)".into()],
            vec!["Check Python version".into(), "Update requirements.txt".into()],
        );
        let lines = cell.display_lines(80);
        // Should have: header + outcome + turns + summary + files section + errors section + next steps
        assert!(lines.len() >= 15, "Expected >= 15 lines, got {}", lines.len());
    }

    #[test]
    fn clarification_cell() {
        let cell = ClarificationCell::new("Which branch?".into());
        assert!(cell.desired_height(80) >= 4); // empty + header + question + hint + empty
    }

    #[test]
    fn approval_cell_pending() {
        let cell = ApprovalCell::new(
            Some("run tests".into()),
            vec!["exec_command".into()],
        );
        assert!(cell.desired_height(80) >= 6); // empty + header + bar + action + cap + waiting + footer
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
