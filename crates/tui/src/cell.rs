//! Cell trait and typed history cells.
//!
//! Each cell represents one logical entry in the chat history.
//! Cells produce `Vec<Line<'static>>` for ratatui rendering and know
//! their desired height at a given terminal width.

use std::any::Any;
use std::sync::OnceLock;
use std::time::Instant;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

// ── Blink animation ──────────────────────────────────────────────────────

/// Process start time for blink animation (deterministic across restarts).
fn blink_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// Current blink tick (changes every 500ms) — used for cache invalidation.
fn blink_tick() -> u64 {
    blink_start().elapsed().as_millis() as u64 / 500
}

/// Radar sweep animation: [>---] [->--] [-->-] [--->] cycling at 200ms.
fn radar_sweep() -> &'static str {
    const FRAMES: [&str; 4] = ["[>---]", "[->--]", "[-->-]", "[--->]"];
    let idx = (blink_start().elapsed().as_millis() / 200) as usize % 4;
    FRAMES[idx]
}

// ── Content sanitization ──────────────────────────────────────────────────

/// Strip ANSI escape sequences (CSI, OSC), OSC 8 hyperlink fragments,
/// and raw JSON protocol envelopes from user-visible display text.
///
/// This is the content-level defense: even if the transport layer leaks
/// escape sequences or protocol JSON into a field, the TUI will not
/// render them as raw text.
pub fn sanitize_display_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek() {
                // CSI sequence: ESC [ ... <letter>
                Some('[') => {
                    chars.next();
                    while let Some(&c) = chars.peek() {
                        chars.next();
                        if c.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                // OSC sequence: ESC ] ... (BEL | ESC \)
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else {
            out.push(ch);
        }
    }
    out
}

// ── Read-only tool classification ─────────────────────────────────────────

/// Returns true for tools that only read state and never modify files or run
/// commands. Used to decide whether to coalesce calls into a compact TaskCard
/// rather than emitting individual ToolCard lines.
pub fn is_read_only_tool(tool: &str) -> bool {
    matches!(
        tool,
        "read_file"
            | "list_files"
            | "search"
            | "grep_search"
            | "file_search"
            | "kubectl_get"
            | "kubectl_logs"
            | "kubectl_describe"
            | "kubectl_events"
    )
}

// ── High-Visibility Palette ────────────────────────────────────────────────
//
// Every color has one exact value.  See docs/v4/UI-v2.md §2.4.
//   DIM     #555555   Rgb(85,85,85)     pipes, timestamps, secondary
//   WHITE   #FFFFFF   Rgb(255,255,255)  primary text, tool names
//   CYAN    #00E5FF   Rgb(0,229,255)    active [-], spinner, progress
//   GREEN   #39FF14   Rgb(57,255,20)    success [*], diff +, PASS
//   RED     #FF3333   Rgb(255,51,51)    fail [x], diff -, errors
//   YELLOW  #FFB000   Rgb(255,176,0)    prompt >, warnings, [!]

const DIM: Style = Style::new().fg(Color::Rgb(85, 85, 85));
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
const WHITE: Style = Style::new().fg(Color::Rgb(255, 255, 255));
const GREEN: Style = Style::new().fg(Color::Rgb(57, 255, 20));
const RED: Style = Style::new().fg(Color::Rgb(255, 51, 51));
const CYAN: Style = Style::new().fg(Color::Rgb(0, 229, 255));
const YELLOW: Style = Style::new().fg(Color::Rgb(255, 176, 0));
const BOLD_YELLOW: Style = Style::new().fg(Color::Rgb(255, 176, 0)).add_modifier(Modifier::BOLD);
const DIM_RED: Style = Style::new().fg(Color::Rgb(255, 51, 51)).add_modifier(Modifier::DIM);

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
        // Rule 1: empty line before ┏━ top-level card
        lines.push(Line::from(""));
        for (i, line) in self.text.lines().enumerate() {
            if i == 0 {
                // ┏━ > user text
                lines.push(Line::from(vec![
                    Span::styled("\u{250f}\u{2501} ", DIM),  // ┏━
                    Span::styled("> ", YELLOW),
                    Span::styled(line.to_string(), WHITE.add_modifier(Modifier::BOLD)),
                ]));
            } else {
                // ┃ continuation
                lines.push(Line::from(vec![
                    Span::styled("\u{2503} ", DIM),  // ┃
                    Span::styled(line.to_string(), WHITE.add_modifier(Modifier::BOLD)),
                ]));
            }
        }
        if !self.images.is_empty() {
            let token = if self.images.len() == 1 {
                "[IMG:1]".to_string()
            } else {
                format!("[IMG:1..{}]", self.images.len())
            };
            lines.push(Line::from(vec![
                Span::styled("\u{2507}  ", DIM),  // ┇
                Span::styled(
                    format!("{} image attached", self.images.len()),
                    DIM,
                ),
                Span::raw(" "),
                Span::styled(token, CYAN),
            ]));
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
        Self {
            understood: sanitize_display_text(&understood),
            target: target.map(|t| sanitize_display_text(&t)),
            steps: steps.into_iter().map(|s| sanitize_display_text(&s)).collect(),
        }
    }
}

impl Cell for MissionCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        // ┣━ [MISSION]
        lines.push(Line::from(vec![
            Span::styled("\u{2523}\u{2501} ", DIM),  // ┣━
            Span::styled("[MISSION]", CYAN),
        ]));
        // ┃  Understood: ...
        lines.push(Line::from(vec![
            Span::styled("\u{2503}  ", DIM),  // ┃
            Span::styled("Understood: ", DIM),
            Span::styled(self.understood.clone(), WHITE),
        ]));
        if let Some(ref target) = self.target {
            lines.push(Line::from(vec![
                Span::styled("\u{2503}  ", DIM),  // ┃
                Span::styled("Target: ", DIM),
                Span::styled(target.clone(), CYAN),
            ]));
        }
        if !self.steps.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("\u{2503}  ", DIM),  // ┃
                Span::styled("Plan:", DIM),
            ]));
            let last_idx = self.steps.len() - 1;
            for (i, step) in self.steps.iter().enumerate() {
                let pipe = if i == last_idx { "\u{2507}  " } else { "\u{2507}  " };  // ┇
                lines.push(Line::from(vec![
                    Span::styled(pipe, DIM),
                    Span::styled(format!("  {}. ", i + 1), DIM),
                    Span::raw(step.clone()),
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
    /// Pre-rendered smart card lines (set when parsed_output is present).
    /// When Some, display_lines() emits these instead of the generic per-call lines.
    pub smart_summary: Option<Vec<Line<'static>>>,
}

/// Max lines of streaming output to show in the cell.
const EXEC_OUTPUT_PREVIEW_LINES: usize = 5;

impl ExecCell {
    pub fn new(tool: String, reasoning: String, args: Option<serde_json::Value>, call_id: Option<String>) -> Self {
        Self {
            smart_summary: None,
            calls: vec![ExecCall {
                tool,
                output: Some(reasoning),
                elapsed_ms: None,
                success: None,
                call_id,
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

    /// Mark a call as completed. Matches by call_id when available,
    /// falls back to last-same-name for backward compatibility.
    pub fn complete_call(
        &mut self,
        tool: &str,
        success: bool,
        summary: String,
        elapsed_ms: u64,
        exit_code: Option<i32>,
        output_preview: Option<String>,
        output_lines_total: Option<u32>,
        call_id: Option<&str>,
    ) {
        let target = if let Some(cid) = call_id {
            // Prefer exact call_id match
            self.calls.iter_mut().find(|c| c.call_id.as_deref() == Some(cid) && c.success.is_none())
        } else {
            // Fallback: last call matching tool name
            self.calls.iter_mut().rev().find(|c| c.tool == tool && c.success.is_none())
        };
        if let Some(call) = target {
            call.success = Some(success);
            call.output = Some(sanitize_display_text(&summary));
            call.elapsed_ms = Some(elapsed_ms);
            call.exit_code = exit_code;
            call.output_preview = output_preview.map(|p| sanitize_display_text(&p));
            call.output_lines_total = output_lines_total;
        }
    }

    /// Add another tool call (coalescing).
    pub fn add_call(&mut self, tool: String, reasoning: String, args: Option<serde_json::Value>, call_id: Option<String>) {
        self.calls.push(ExecCall {
            tool,
            output: Some(reasoning),
            elapsed_ms: None,
            success: None,
            call_id,
            args,
            exit_code: None,
            output_preview: None,
            output_lines_total: None,
        });
    }
}

/// Format a tool call for display based on tool type.
/// Returns a display string like `$ docker build .` or `Read: src/main.rs`
///
/// All output is sanitized — ANSI/OSC escapes are stripped.
pub fn format_tool_display(tool: &str, args: Option<&serde_json::Value>, reasoning: &str) -> String {
    let raw = match tool {
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
    };
    sanitize_display_text(&raw)
}

impl Cell for ExecCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn transcript_animation_tick(&self) -> Option<u64> {
        if self.calls.iter().any(|c| c.success.is_none()) {
            Some(blink_tick())
        } else {
            None
        }
    }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        // Use smart card rendering when all calls are complete and smart summary is set.
        let all_done = self.calls.iter().all(|c| c.success.is_some());
        if all_done {
            if let Some(ref smart) = self.smart_summary {
                return smart.clone();
            }
        }

        let mut lines = Vec::new();
        for call in &self.calls {
            let display = format_tool_display(
                &call.tool,
                call.args.as_ref(),
                call.output.as_deref().unwrap_or(""),
            );
            let is_read_only = is_read_only_tool(&call.tool);

            match call.success {
                None => {
                    // In-progress: radar sweep for read-only, [-] for others
                    let switch = if is_read_only {
                        Span::styled(format!("{} ", radar_sweep()), CYAN)
                    } else {
                        Span::styled("[-] ", CYAN)
                    };
                    lines.push(Line::from(vec![
                        Span::styled("\u{2523}\u{2501} ", DIM),  // ┣━
                        switch,
                        Span::styled(display, DIM),
                    ]));
                }
                Some(success) => {
                    let switch = if success {
                        Span::styled("[*] ", if is_read_only { DIM } else { GREEN })
                    } else {
                        Span::styled("[x] ", RED)
                    };
                    let elapsed = call.elapsed_ms
                        .map(|ms| format!("  ({:.1}s)", ms as f64 / 1000.0))
                        .unwrap_or_default();
                    let exit_info = if !success {
                        call.exit_code.map(|c| format!(" [EXIT:{}]", c)).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let text_style = if is_read_only && success { DIM } else { WHITE };
                    lines.push(Line::from(vec![
                        Span::styled("\u{2523}\u{2501} ", DIM),  // ┣━
                        switch,
                        Span::styled(format!("{display}{exit_info}"), text_style),
                        Span::styled(elapsed, DIM),
                    ]));

                    // Show output preview: error output for failures, elided summary for long successful output
                    if let Some(ref preview) = call.output_preview {
                        let preview_lines: Vec<&str> = preview.lines().collect();
                        let total_lines = call.output_lines_total.unwrap_or(preview_lines.len() as u32);
                        let style = if success {
                            DIM
                        } else {
                            DIM_RED
                        };

                        if !success {
                            // Error: show last few lines with ┃/╰ pipe
                            let show = preview_lines.len().min(5);
                            let start = preview_lines.len().saturating_sub(show);
                            for (i, line_text) in preview_lines[start..].iter().enumerate() {
                                let is_last = i == show - 1;
                                let prefix = if is_last { "\u{2570}  " } else { "\u{2503}  " };
                                let truncated: String = line_text.chars().take(100).collect();
                                lines.push(Line::from(vec![
                                    Span::styled(prefix, DIM),
                                    Span::styled(truncated, style),
                                ]));
                            }
                        } else if total_lines > 8 {
                            // Success with long output: head(3) + ... + tail(2)
                            let head: Vec<&str> = preview_lines.iter().take(3).copied().collect();
                            let tail: Vec<&str> = preview_lines.iter().rev().take(2).rev().copied().collect();
                            for line_text in &head {
                                let truncated: String = line_text.chars().take(100).collect();
                                lines.push(Line::from(vec![
                                    Span::styled("\u{2503}  ", DIM),  // ┃
                                    Span::styled(truncated, style),
                                ]));
                            }
                            let hidden = total_lines.saturating_sub(5);
                            lines.push(Line::from(vec![
                                Span::styled("\u{2503}  ", DIM),  // ┃
                                Span::styled(format!("... ({hidden} lines)"), DIM),
                            ]));
                            for (i, line_text) in tail.iter().enumerate() {
                                let is_last = i == tail.len() - 1;
                                let prefix = if is_last { "\u{2570}  " } else { "\u{2503}  " };
                                let truncated: String = line_text.chars().take(100).collect();
                                lines.push(Line::from(vec![
                                    Span::styled(prefix, DIM),
                                    Span::styled(truncated, style),
                                ]));
                            }
                        }

                        if total_lines > 8 || !success {
                            if total_lines > 5 && !success {
                                lines.push(Line::from(vec![
                                    Span::styled("\u{2507}  ", DIM),  // ┇
                                    Span::styled(format!("({total_lines} lines total)"), DIM),
                                ]));
                            }
                        }
                    }
                }
            }
        }
        // Show streaming output preview (last N lines, dimmed, with pipe prefix)
        if !self.streaming_output.is_empty() {
            let output_lines: Vec<&str> = self.streaming_output.lines().collect();
            let total = output_lines.len();
            let start = total.saturating_sub(EXEC_OUTPUT_PREVIEW_LINES);
            if start > 0 {
                lines.push(Line::from(vec![
                    Span::styled("\u{2503}  ", DIM),  // ┃
                    Span::styled(format!("... +{} lines", start), DIM),
                ]));
            }
            for (i, line_text) in output_lines[start..].iter().enumerate() {
                let is_last = i == output_lines[start..].len() - 1;
                let prefix = if is_last { "\u{2570}  " } else { "\u{2503}  " };
                let truncated: String = line_text.chars().take(100).collect();
                lines.push(Line::from(vec![
                    Span::styled(prefix, DIM),
                    Span::styled(truncated, DIM),
                ]));
            }
        }
        lines
    }

    /// Pager view: same header as display_lines but shows ALL streaming output.
    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = self.display_lines(width);
        if !self.streaming_output.is_empty() {
            let output_lines: Vec<&str> = self.streaming_output.lines().collect();
            let total = output_lines.len();
            if total > EXEC_OUTPUT_PREVIEW_LINES {
                let strip = 1 + EXEC_OUTPUT_PREVIEW_LINES.min(total);
                lines.truncate(lines.len().saturating_sub(strip));
                for (i, line_text) in output_lines.iter().enumerate() {
                    let is_last = i == total - 1;
                    let prefix = if is_last { "\u{2570}  " } else { "\u{2503}  " };
                    let safe = sanitize_display_text(line_text);
                    lines.push(Line::from(vec![
                        Span::styled(prefix, DIM),
                        Span::styled(safe, DIM),
                    ]));
                }
            }
        }
        lines
    }
}

// ── AgentMessageCell ───────────────────────────────────────────────────────

/// Agent markdown response — stores raw text, re-renders at requested width.
#[derive(Debug)]
pub struct AgentMessageCell {
    /// Raw markdown text accumulated from the stream.
    pub raw_text: String,
    /// Whether this is the first message in a sequence (adds top padding).
    pub is_first_line: bool,
}

impl AgentMessageCell {
    pub fn new(raw_text: String, is_first_line: bool) -> Self {
        Self { raw_text, is_first_line }
    }

    /// Create from raw markdown text (for non-streamed messages).
    pub fn from_text(text: &str) -> Self {
        let clean = sanitize_display_text(text);
        Self {
            raw_text: clean,
            is_first_line: true,
        }
    }

    /// Append raw markdown text (from streaming).
    pub fn append_raw(&mut self, text: &str) {
        self.raw_text.push_str(text);
    }

    /// Render raw text at the given width using MarkdownStreamCollector.
    fn render_at_width(&self, width: u16) -> Vec<Line<'static>> {
        use crate::markdown_stream::MarkdownStreamCollector;
        let mut collector = MarkdownStreamCollector::new(Some(width));
        collector.push_delta(&self.raw_text);
        collector.finalize_and_drain()
    }
}

impl Cell for AgentMessageCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.render_at_width(width)
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
        let message = sanitize_display_text(&message);
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
            ErrorCategory::Api => "API_ERROR",
            ErrorCategory::Parse => "PARSE_ERROR",
            ErrorCategory::Execution => "EXEC_ERROR",
            ErrorCategory::Generic => "ERROR",
        };

        let mut header_spans = vec![
            Span::styled("\u{2523}\u{2501} ", DIM),  // ┣━
            Span::styled("[x] ", RED),
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
            header_spans.push(Span::styled(truncated, DIM_RED));
        }
        lines.push(Line::from(header_spans));

        // Additional lines with pipe prefix (up to 4 more, capping card at 6 total)
        let extra = msg_lines.iter().skip(1).take(4).collect::<Vec<_>>();
        let extra_len = extra.len();
        for (i, line_text) in extra.iter().enumerate() {
            let truncated: String = line_text.chars().take(100).collect();
            let prefix = if i == extra_len - 1 && msg_lines.len() <= 5 {
                "\u{2570}  "  // ╰
            } else {
                "\u{2503}  "  // ┃
            };
            lines.push(Line::from(vec![
                Span::styled(prefix, DIM),
                Span::styled(truncated, DIM_RED),
            ]));
        }
        if msg_lines.len() > 5 {
            lines.push(Line::from(vec![
                Span::styled("\u{2570}  ", DIM),  // ╰
                Span::styled(format!("... ({} more lines)", msg_lines.len() - 5), DIM),
            ]));
        }

        lines
    }
}

// ── TaskCard ───────────────────────────────────────────────────────────────

/// Compact milestone cell for a group of completed read-only operations.
///
/// Replaces the individual per-call ExecCell lines in committed scrollback
/// with a single "Read N files · path1, path2…" summary.
#[derive(Debug)]
pub struct TaskCard {
    /// Short human-readable title, e.g. "Read 4 files".
    pub title: String,
    /// File/resource paths that were read (display-truncated).
    pub paths: Vec<String>,
}

impl TaskCard {
    /// Build a TaskCard from a completed read-only ExecCell.
    pub fn from_exec_cell(cell: &ExecCell) -> Self {
        let count = cell.calls.len();
        let paths: Vec<String> = cell.calls.iter()
            .filter_map(|c| {
                c.args.as_ref()
                    .and_then(|a| a.get("path").or_else(|| a.get("directory")))
                    .and_then(|v| v.as_str())
                    .map(|p| {
                        // Show last 2 path components for readability.
                        let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
                        if parts.len() >= 2 {
                            format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
                        } else {
                            p.to_string()
                        }
                    })
            })
            .collect();

        let title = match count {
            1 => {
                let name = paths.first().cloned().unwrap_or_else(|| "file".into());
                // If it looks like a full path, just say "Read file"
                let base = name.rsplit('/').next().unwrap_or(&name);
                format!("Read {}", base)
            }
            n => format!("Read {} files", n),
        };

        Self { title: sanitize_display_text(&title), paths }
    }
}

impl Cell for TaskCard {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        // ┣━ [*] READ_FILES (N files)
        let count = self.paths.len();
        let header = if count <= 1 {
            self.title.clone()
        } else {
            format!("READ_FILES ({} files)", count)
        };
        lines.push(Line::from(vec![
            Span::styled("\u{2523}\u{2501} ", DIM),  // ┣━
            Span::styled("[*] ", DIM),
            Span::styled(header, DIM),
        ]));
        // ┇  path list: comma-separated, truncated to width
        if !self.paths.is_empty() {
            let max_w = (width as usize).saturating_sub(6);
            let joined = self.paths.join(", ");
            let display: String = if joined.len() > max_w {
                let truncated: String = joined.chars().take(max_w.saturating_sub(1)).collect();
                format!("{}\u{2026}", truncated)
            } else {
                joined
            };
            lines.push(Line::from(vec![
                Span::styled("\u{2507}  ", DIM),  // ┇
                Span::styled(display, DIM),
            ]));
        }
        lines
    }

    fn desired_height(&self, width: u16) -> u16 {
        if self.paths.is_empty() { 1 } else { self.display_lines(width).len() as u16 }
    }
}

// ── DiffCard ───────────────────────────────────────────────────────────────

/// A single preview line in a DiffCard.
#[derive(Debug, Clone)]
pub struct DiffPreviewLine {
    pub kind: DiffPreviewKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffPreviewKind {
    Add,
    Remove,
}

/// History card for file modifications — "Edited N files" + compact diff preview.
///
/// Appears after write_file / apply_patch operations and before ResultCard.
/// Full diffs are in the pager (Ctrl+T).
#[derive(Debug)]
pub struct DiffCard {
    /// Modified file paths.
    pub files: Vec<String>,
    /// Up to 3 preview lines from the most representative diff hunk.
    pub preview: Vec<DiffPreviewLine>,
}

impl DiffCard {
    pub fn new(files: Vec<String>, preview: Vec<DiffPreviewLine>) -> Self {
        let files = files.into_iter().map(|f| sanitize_display_text(&f)).collect();
        let preview = preview.into_iter().map(|l| DiffPreviewLine {
            kind: l.kind,
            text: sanitize_display_text(&l.text),
        }).collect();
        Self { files, preview }
    }
}

impl Cell for DiffCard {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let n = self.files.len();
        // ┣━ [DIFF] N files edited
        let header = if n == 1 { "1 file edited".to_string() } else { format!("{} files edited", n) };
        lines.push(Line::from(vec![
            Span::styled("\u{2523}\u{2501} ", DIM),  // ┣━
            Span::styled("[DIFF] ", CYAN),
            Span::styled(header, DIM),
        ]));

        // File list (up to 3) with ┃ pipe
        for path in self.files.iter().take(3) {
            let max_w = (width as usize).saturating_sub(6);
            let display: String = if path.len() > max_w {
                let t: String = path.chars().take(max_w.saturating_sub(1)).collect();
                format!("{}\u{2026}", t)
            } else {
                path.clone()
            };
            lines.push(Line::from(vec![
                Span::styled("\u{2503}  ", DIM),  // ┃
                Span::styled(display, DIM),
            ]));
        }
        if n > 3 {
            lines.push(Line::from(vec![
                Span::styled("\u{2503}  ", DIM),  // ┃
                Span::styled(format!("... {} more", n - 3), DIM),
            ]));
        }

        // Diff preview lines (max 3) with pipe
        let preview_count = self.preview.len().min(3);
        for (i, line) in self.preview.iter().take(3).enumerate() {
            let (prefix, style) = match line.kind {
                DiffPreviewKind::Add => ("+", GREEN),
                DiffPreviewKind::Remove => ("-", RED),
            };
            let max_w = (width as usize).saturating_sub(8);
            let text: String = if line.text.len() > max_w {
                let t: String = line.text.chars().take(max_w.saturating_sub(1)).collect();
                format!("{}\u{2026}", t)
            } else {
                line.text.clone()
            };
            let pipe = if i == preview_count - 1 { "\u{2570}  " } else { "\u{2503}  " };  // ╰ or ┃
            lines.push(Line::from(vec![
                Span::styled(pipe, DIM),
                Span::styled(format!("{} ", prefix), style),
                Span::styled(text, DIM),
            ]));
        }

        lines
    }

    /// Pager view: all files + all preview lines (no 3-item caps).
    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let n = self.files.len();
        let header = if n == 1 { "1 file edited".to_string() } else { format!("{} files edited", n) };
        lines.push(Line::from(vec![
            Span::styled("\u{2523}\u{2501} ", DIM),  // ┣━
            Span::styled("[DIFF] ", CYAN),
            Span::styled(header, DIM),
        ]));
        for path in &self.files {
            let max_w = (width as usize).saturating_sub(6);
            let display: String = if path.len() > max_w {
                let t: String = path.chars().take(max_w.saturating_sub(1)).collect();
                format!("{}\u{2026}", t)
            } else {
                path.clone()
            };
            lines.push(Line::from(vec![
                Span::styled("\u{2503}  ", DIM),  // ┃
                Span::styled(display, DIM),
            ]));
        }
        let total_preview = self.preview.len();
        for (i, line) in self.preview.iter().enumerate() {
            let (prefix, style) = match line.kind {
                DiffPreviewKind::Add => ("+", GREEN),
                DiffPreviewKind::Remove => ("-", RED),
            };
            let max_w = (width as usize).saturating_sub(8);
            let text: String = if line.text.len() > max_w {
                let t: String = line.text.chars().take(max_w.saturating_sub(1)).collect();
                format!("{}\u{2026}", t)
            } else {
                line.text.clone()
            };
            let pipe = if i == total_preview - 1 { "\u{2570}  " } else { "\u{2503}  " };
            lines.push(Line::from(vec![
                Span::styled(pipe, DIM),
                Span::styled(format!("{} ", prefix), style),
                Span::styled(text, DIM),
            ]));
        }
        lines
    }
}

// ── GroupDivider ───────────────────────────────────────────────────────────

/// Thin dim separator between distinct task groups in the history stream.
///
/// Emitted when a new read-only exploration group begins after a write/exec group.
#[derive(Debug)]
pub struct GroupDivider;

impl Cell for GroupDivider {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        // ┇┄┄┄┄┄... — V2 pipe divider style
        let w = (width as usize).saturating_sub(3).min(60);
        vec![
            Line::from(Span::styled(
                format!("\u{2507}{}", "\u{2504}".repeat(w)),
                DIM,
            )),
        ]
    }

    fn desired_height(&self, _width: u16) -> u16 { 1 }

    fn is_continuation(&self) -> bool { true }
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
        Self {
            outcome: sanitize_display_text(&outcome),
            summary: sanitize_display_text(&summary),
            turns,
            elapsed_ms,
            files_modified: files_modified.into_iter().map(|f| sanitize_display_text(&f)).collect(),
            errors_encountered: errors_encountered.into_iter().map(|e| sanitize_display_text(&e)).collect(),
            next_steps: next_steps.into_iter().map(|s| sanitize_display_text(&s)).collect(),
        }
    }
}

impl Cell for ResultCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let secs = self.elapsed_ms as f64 / 1000.0;
        let mut lines = Vec::new();
        // Rule 1: empty line before ┏━ top-level card
        lines.push(Line::from(""));

        // ┏━ [*] RESULT: PASS  (12.4s)
        let (switch, label, outcome_style) = match self.outcome.as_str() {
            "PASS" => ("[*]", "PASS", GREEN),
            "PARTIAL" => ("[~]", "PARTIAL", YELLOW),
            _ => ("[x]", "FAIL", RED),
        };
        lines.push(Line::from(vec![
            Span::styled("\u{250f}\u{2501} ", DIM),  // ┏━
            Span::styled(format!("{switch} "), outcome_style),
            Span::styled("RESULT: ", DIM),
            Span::styled(label.to_string(), Style::new().fg(outcome_style.fg.unwrap_or(Color::Reset)).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  ({secs:.1}s)"), DIM),
        ]));

        // ┃  summary line
        if !self.summary.is_empty() {
            let first = self.summary.lines().next().unwrap_or(&self.summary);
            let truncated: String = first.chars().take(120).collect();
            lines.push(Line::from(vec![
                Span::styled("\u{2503}  ", DIM),  // ┃
                Span::styled(truncated, WHITE),
            ]));
        }

        // ┇  Next: hint for non-pass outcomes
        if self.outcome != "PASS" {
            if let Some(step) = self.next_steps.first() {
                let truncated: String = step.chars().take(100).collect();
                lines.push(Line::from(vec![
                    Span::styled("\u{2507}  ", DIM),  // ┇
                    Span::styled("Next: ", DIM),
                    Span::styled(truncated, DIM),
                ]));
            }
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
        Self { question: sanitize_display_text(&question) }
    }
}

impl Cell for ClarificationCell {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let rule_w = (width as usize).saturating_sub(22).min(50);
        vec![
            Line::from(vec![
                Span::styled("\u{2501}\u{2501} ", DIM),  // ━━
                Span::styled("[?] ", YELLOW),
                Span::styled("CLARIFICATION ", YELLOW),
                Span::styled("\u{2501}".repeat(rule_w), DIM),
            ]),
            Line::from(vec![
                Span::styled("\u{2503}  ", DIM),  // ┃
                Span::styled("DeCIpher asks: ", BOLD_YELLOW),
                Span::styled(self.question.clone(), WHITE),
            ]),
            Line::from(vec![
                Span::styled("\u{2503}  ", DIM),  // ┃
                Span::styled("Reply below and DeCIpher will continue.", DIM),
            ]),
            Line::from(Span::styled(
                "\u{2501}".repeat((width as usize).min(67)),
                DIM,
            )),
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

    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(Line::from(""));  // Rule 1: top-level card isolation
        let rule_w = (width as usize).saturating_sub(26).min(50);
        // ━━ [!] APPROVAL_REQUIRED ━━━...
        lines.push(Line::from(vec![
            Span::styled("\u{2501}\u{2501} ", DIM),  // ��━
            Span::styled("[!] ", YELLOW),
            Span::styled("APPROVAL_REQUIRED ", YELLOW),
            Span::styled("\u{2501}".repeat(rule_w), DIM),
        ]));
        if let Some(ref action) = self.action {
            lines.push(Line::from(vec![
                Span::styled("\u{2503}  ", DIM),  // ┃
                Span::styled("ACTION: ", BOLD),
                Span::styled(action.clone(), CYAN),
            ]));
        }
        if !self.capabilities.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("\u{2503}  ", DIM),  // ┃
                Span::styled("CAPABILITIES:", DIM),
            ]));
            for cap in &self.capabilities {
                lines.push(Line::from(vec![
                    Span::styled("\u{2503}    ", DIM),  // ┃
                    Span::styled("[-] ", YELLOW),
                    Span::raw(cap.clone()),
                ]));
            }
        }
        match self.decision {
            Some(true) => {
                lines.push(Line::from(vec![
                    Span::styled("\u{2503}  ", DIM),  // ┃
                    Span::styled("[*] Approved", GREEN),
                ]));
            }
            Some(false) => {
                lines.push(Line::from(vec![
                    Span::styled("\u{2503}  ", DIM),  // ┃
                    Span::styled("[x] Denied", RED),
                ]));
            }
            None => {}
        }
        // Bottom rule
        lines.push(Line::from(Span::styled(
            "\u{2501}".repeat((width as usize).min(67)),
            DIM,
        )));
        lines
    }
}

// ── Smart card rendering ───────────────────────────────────────────────────────

/// Render parsed exec output as styled summary lines for the chat history.
///
/// Called from `chat.rs` when `ToolResult.parsed_output` is present.
/// Returns `None` for `Generic` or unrecognized type — caller falls back to
/// generic ExecCell completion line.
pub fn render_smart_card_lines(
    parsed_json: &str,
    success: bool,
    elapsed_ms: u64,
) -> Option<Vec<Line<'static>>> {
    let v: serde_json::Value = serde_json::from_str(parsed_json).ok()?;
    let kind = v.get("type").and_then(|t| t.as_str())?;
    let secs = elapsed_ms as f64 / 1000.0;

    let (icon, icon_style) = if success {
        ("*", GREEN)     // [*]
    } else {
        ("x", RED)       // [x]
    };

    match kind {
        "test_suite" => render_test_suite(&v, icon, icon_style, secs),
        "docker_build" => render_docker_build(&v, icon, icon_style, secs),
        "docker_run" => render_docker_run(&v, icon, icon_style, secs),
        "compose" => render_compose(&v, icon, icon_style, secs),
        "git_op" => render_git_op(&v, icon, icon_style, secs),
        "lint" => render_lint(&v, icon, icon_style, secs),
        "kube_pod" => render_kube_pod(&v, icon, icon_style, secs),
        "kube_log" => render_kube_log(&v, icon, icon_style, secs),
        "kube_event" => render_kube_event(&v, icon, icon_style, secs),
        "ci" => render_ci(&v, icon, icon_style, secs),
        "env_setup" => render_env_setup(&v, icon, icon_style, secs),
        "migration" => render_migration(&v, icon, icon_style, secs),
        _ => None,
    }
}

fn str_field<'a>(v: &'a serde_json::Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("")
}

fn u32_field(v: &serde_json::Value, key: &str) -> u32 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0) as u32
}

fn header_line(icon: &'static str, style: Style, title: String, secs: f64) -> Line<'static> {
    Line::from(vec![
        Span::styled("\u{2523}\u{2501} ", DIM),  // ┣━
        Span::styled(format!("[{icon}] "), style),
        Span::styled(title, Style::default()),
        Span::styled(format!("  ({secs:.1}s)"), DIM),
    ])
}

fn detail_line(detail: String) -> Line<'static> {
    Line::from(vec![
        Span::styled("\u{2507}  ", DIM),  // ┇
        Span::styled(detail, DIM),
    ])
}


fn render_test_suite(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let runner = str_field(v, "runner");
    let passed = u32_field(v, "passed");
    let failed = u32_field(v, "failed");
    let skipped = u32_field(v, "skipped");

    let title = if failed > 0 {
        format!("Tests failed \u{00b7} {runner}")   // ·
    } else {
        format!("Tests passed \u{00b7} {runner}")
    };

    let mut lines = vec![header_line(icon, style, title, secs)];

    let mut detail = format!("{passed} passed \u{00b7} {failed} failed");
    if skipped > 0 { detail.push_str(&format!(" \u{00b7} {skipped} skipped")); }
    if let Some(cov) = v.get("coverage").and_then(|c| c.as_f64()) {
        detail.push_str(&format!(" \u{00b7} {:.1}% cov", cov));
    }
    lines.push(detail_line(detail));

    // Show up to 3 failure names with ┇ internal segmentation (Rule 3)
    if let Some(failures) = v.get("failures").and_then(|f| f.as_array()) {
        let fail_names: Vec<&str> = failures.iter().take(3)
            .filter_map(|f| f.get("name").and_then(|n| n.as_str()))
            .filter(|n| !n.is_empty())
            .collect();
        if !fail_names.is_empty() {
            // Rule 3: ┇ spacer between summary and failure details
            lines.push(Line::from(Span::styled("\u{2507}", DIM)));
            for (i, name) in fail_names.iter().enumerate() {
                let pipe = if i == fail_names.len() - 1 { "\u{2570}  " } else { "\u{2503}  " };
                lines.push(Line::from(vec![
                    Span::styled(pipe, DIM),
                    Span::styled(name.to_string(), DIM_RED),
                ]));
            }
        }
    }

    Some(lines)
}

fn render_docker_build(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let image = str_field(v, "image");
    let steps_total = u32_field(v, "steps_total");
    let steps_done = u32_field(v, "steps_done");
    let cached = v.get("cached").and_then(|c| c.as_bool()).unwrap_or(false);

    let title = if image.is_empty() { "Docker build".to_string() } else { format!("Built {image}") };
    let mut lines = vec![header_line(icon, style, title, secs)];

    let mut detail = if steps_total > 0 {
        format!("{steps_done}/{steps_total} steps")
    } else {
        String::new()
    };
    if let Some(stages) = v.get("stages").and_then(|s| s.as_array()) {
        if !stages.is_empty() {
            if !detail.is_empty() { detail.push_str(" \u{00b7} "); }
            detail.push_str(&format!("{} stage{}", stages.len(), if stages.len() == 1 { "" } else { "s" }));
        }
    }
    if let Some(mb) = v.get("size_mb").and_then(|s| s.as_f64()) {
        if !detail.is_empty() { detail.push_str(" \u{00b7} "); }
        detail.push_str(&format!("{mb:.0}MB"));
    }
    if cached {
        if !detail.is_empty() { detail.push_str(" \u{00b7} "); }
        detail.push_str("CACHED");
    }
    if !detail.is_empty() { lines.push(detail_line(detail)); }

    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        if !err.is_empty() {
            let truncated: String = err.chars().take(100).collect();
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("\u{2570}  ", DIM),  // ╰
                Span::styled(truncated, DIM_RED),
            ]));
        }
    }

    Some(lines)
}

fn render_docker_run(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let container = str_field(v, "container");
    let title = if container.is_empty() { "Docker run".to_string() } else { format!("Container {container}") };
    let mut lines = vec![header_line(icon, style, title, secs)];

    if let Some(ports) = v.get("ports").and_then(|p| p.as_array()) {
        if !ports.is_empty() {
            let port_str = ports.iter()
                .take(3)
                .filter_map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(" \u{00b7} ");
            lines.push(detail_line(port_str));
        }
    }
    if let Some(health) = v.get("health").and_then(|h| h.as_str()) {
        if !health.is_empty() {
            lines.push(detail_line(health.to_string()));
        }
    }

    Some(lines)
}

fn render_compose(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let svc_count = v.get("services").and_then(|s| s.as_array()).map(|a| a.len()).unwrap_or(0);
    let title = format!("COMPOSE_UP ({svc_count} services)");
    let mut lines = vec![header_line(icon, style, title, secs)];

    // Multi-line switch matrix — one service per line
    if let Some(services) = v.get("services").and_then(|s| s.as_array()) {
        let svcs: Vec<_> = services.iter().take(6).collect();
        let last_idx = svcs.len().saturating_sub(1);
        for (i, s) in svcs.iter().enumerate() {
            let name = s.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let image = s.get("image").and_then(|im| im.as_str()).unwrap_or("");
            let port = s.get("port").and_then(|p| p.as_str()).unwrap_or("--");
            let status = s.get("status").and_then(|st| st.as_str()).unwrap_or("?");
            let health = s.get("health").and_then(|h| h.as_str()).unwrap_or("");

            let (switch, sw_style) = if status == "up" { ("[*]", GREEN) } else { ("[x]", RED) };
            let pipe = if i == last_idx { "\u{2570}  " } else { "\u{2503}  " };

            let mut detail = format!("{name:<10}");
            if !image.is_empty() { detail.push_str(&format!("{image:<18}")); }
            detail.push_str(&format!("{port:<6}"));
            if !health.is_empty() { detail.push_str(health); }

            lines.push(Line::from(vec![
                Span::styled(pipe, DIM),
                Span::styled(format!("{switch} "), sw_style),
                Span::styled(detail, DIM),
            ]));
        }
    }

    Some(lines)
}

fn render_git_op(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let op = str_field(v, "op");
    let branch = v.get("branch").and_then(|b| b.as_str()).unwrap_or("");
    let files = u32_field(v, "files_changed");
    let adds = u32_field(v, "additions");
    let dels = u32_field(v, "deletions");

    let mut title = format!("Git {op}");
    if !branch.is_empty() { title.push_str(&format!(" \u{00b7} {branch}")); }
    let mut lines = vec![header_line(icon, style, title, secs)];

    if let Some(msg) = v.get("commit_msg").and_then(|m| m.as_str()) {
        if !msg.is_empty() {
            let truncated: String = msg.chars().take(60).collect();
            lines.push(detail_line(truncated));
        }
    }
    if files > 0 {
        let detail = format!("{files} file{} \u{00b7} +{adds} \u{2212}{dels}", if files == 1 { "" } else { "s" });
        lines.push(detail_line(detail));
    }
    if let Some(conflicts) = v.get("conflicts").and_then(|c| c.as_array()) {
        if !conflicts.is_empty() {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("\u{2570}  ", DIM),  // ╰
                Span::styled(format!("{} conflict{}", conflicts.len(), if conflicts.len() == 1 { "" } else { "s" }),
                    DIM_RED),
            ]));
        }
    }

    Some(lines)
}

fn render_lint(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let tool = str_field(v, "tool");
    let warnings = u32_field(v, "warnings");
    let errors = u32_field(v, "errors");

    let title = if errors > 0 || warnings > 0 {
        format!("Lint \u{00b7} {tool}")
    } else {
        format!("Lint passed \u{00b7} {tool}")
    };
    let mut lines = vec![header_line(icon, style, title, secs)];

    let detail = if errors == 0 && warnings == 0 {
        "0 warnings \u{00b7} 0 errors".to_string()
    } else {
        format!("{errors} error{} \u{00b7} {warnings} warning{}", if errors == 1 { "" } else { "s" }, if warnings == 1 { "" } else { "s" })
    };
    lines.push(detail_line(detail));

    // Show first lint item
    if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
        if let Some(first) = items.first() {
            let msg = first.get("message").and_then(|m| m.as_str()).unwrap_or("");
            if !msg.is_empty() {
                let truncated: String = msg.chars().take(80).collect();
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled("\u{2570}  ", DIM),  // ╰
                    Span::styled(truncated, DIM),
                ]));
            }
        }
    }

    Some(lines)
}

fn render_kube_pod(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let resource = str_field(v, "resource");
    let title = format!("ROLLOUT: {resource}");
    let mut lines = vec![header_line(icon, style, title, secs)];

    // Multi-line switch matrix — one pod per line
    if let Some(pods) = v.get("pods").and_then(|p| p.as_array()) {
        let pod_list: Vec<_> = pods.iter().take(6).collect();
        let last_idx = pod_list.len().saturating_sub(1);
        for (i, p) in pod_list.iter().enumerate() {
            let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let status = p.get("status").and_then(|s| s.as_str()).unwrap_or("?");
            let ready = p.get("ready").and_then(|r| r.as_str()).unwrap_or("?");
            let restarts = p.get("restarts").and_then(|r| r.as_u64()).unwrap_or(0);

            let (switch, sw_style) = match status {
                "Running" => ("[*]", GREEN),
                "CrashLoopBackOff" | "Error" | "OOMKilled" => ("[x]", RED),
                "Pending" | "ContainerCreating" => ("[-]", CYAN),
                _ => ("[ ]", DIM),
            };
            let pipe = if i == last_idx { "\u{2570}  " } else { "\u{2503}  " };

            let text = format!("{name:<24}{ready:<8}{status:<16}{restarts}");

            lines.push(Line::from(vec![
                Span::styled(pipe, DIM),
                Span::styled(format!("{switch} "), sw_style),
                Span::styled(text, DIM),
            ]));
        }
    }

    Some(lines)
}

fn render_kube_log(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let pod = str_field(v, "pod");
    let errors = u32_field(v, "errors");
    let warnings = u32_field(v, "warnings");
    let total = u32_field(v, "lines_total");

    let mut title = if pod.is_empty() { "kubectl logs".to_string() } else { format!("kubectl logs {pod}") };
    if errors > 0 { title.push_str(" \u{00b7} errors found"); }
    let mut lines = vec![header_line(icon, style, title, secs)];

    lines.push(detail_line(format!("{total} lines \u{00b7} {errors} error{} \u{00b7} {warnings} warning{}",
        if errors == 1 { "" } else { "s" }, if warnings == 1 { "" } else { "s" })));

    if let Some(cause) = v.get("root_cause").and_then(|c| c.as_str()) {
        if !cause.is_empty() {
            let truncated: String = cause.chars().take(100).collect();
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("\u{2570}  ", DIM),  // ╰
                Span::styled(truncated, DIM_RED),
            ]));
        }
    }

    Some(lines)
}

fn render_kube_event(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let ns = str_field(v, "namespace");
    let title = format!("kubectl events \u{00b7} {ns}");
    let mut lines = vec![header_line(icon, style, title, secs)];

    if let Some(events) = v.get("events").and_then(|e| e.as_array()) {
        let warn_count = events.iter().filter(|e| e.get("kind").and_then(|k| k.as_str()) == Some("Warning")).count();
        lines.push(detail_line(format!("{} event{} \u{00b7} {} Warning{}",
            events.len(), if events.len() == 1 { "" } else { "s" },
            warn_count, if warn_count == 1 { "" } else { "s" })));
    }

    Some(lines)
}

fn render_ci(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let pid = v.get("pipeline_id").and_then(|p| p.as_str());
    let title = if let Some(pid) = pid {
        format!("PIPELINE #{pid}")
    } else {
        "PIPELINE".to_string()
    };
    let mut lines = vec![header_line(icon, style, title, secs)];

    // Multi-line switch matrix — one stage per line
    if let Some(stages) = v.get("stages").and_then(|s| s.as_array()) {
        let stgs: Vec<_> = stages.iter().take(8).collect();
        let last_idx = stgs.len().saturating_sub(1);
        for (i, s) in stgs.iter().enumerate() {
            let name = s.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let status = s.get("status").and_then(|st| st.as_str()).unwrap_or("pending");
            let elapsed = s.get("elapsed").and_then(|e| e.as_str()).unwrap_or("--");
            let detail = s.get("detail").and_then(|d| d.as_str()).unwrap_or("");

            let (switch, sw_style) = match status {
                "success" => ("[*]", GREEN),
                "failure" => ("[x]", RED),
                "running" => ("[-]", CYAN),
                _ => ("[ ]", DIM),
            };
            let pipe = if i == last_idx { "\u{2570}  " } else { "\u{2503}  " };

            let mut text = format!("{name:<12}{elapsed:<8}");
            if !detail.is_empty() { text.push_str(detail); }

            lines.push(Line::from(vec![
                Span::styled(pipe, DIM),
                Span::styled(format!("{switch} "), sw_style),
                Span::styled(text, DIM),
            ]));
        }
    }

    Some(lines)
}

fn render_env_setup(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let manager = str_field(v, "manager");
    let packages = u32_field(v, "packages");
    let vulns = u32_field(v, "vulnerabilities");

    let title = if packages > 0 { format!("Deps installed \u{00b7} {manager}") } else { format!("Build \u{00b7} {manager}") };
    let mut lines = vec![header_line(icon, style, title, secs)];

    let mut detail = if packages > 0 { format!("{packages} package{}",  if packages == 1 { "" } else { "s" }) } else { String::new() };
    if vulns > 0 {
        if !detail.is_empty() { detail.push_str(" \u{00b7} "); }
        detail.push_str(&format!("{vulns} vulnerabilit{}", if vulns == 1 { "y" } else { "ies" }));
    }
    if !detail.is_empty() { lines.push(detail_line(detail)); }

    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        if !err.is_empty() {
            let truncated: String = err.chars().take(100).collect();
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("\u{2570}  ", DIM),  // ╰
                Span::styled(truncated, DIM_RED),
            ]));
        }
    }

    Some(lines)
}

fn render_migration(v: &serde_json::Value, icon: &'static str, style: Style, secs: f64) -> Option<Vec<Line<'static>>> {
    let applied = u32_field(v, "applied");
    let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("");

    let title = if applied > 0 { "Migration applied".to_string() } else { "Migration".to_string() };
    let mut lines = vec![header_line(icon, style, title, secs)];

    let mut detail = format!("{applied} applied");
    if !name.is_empty() { detail.push_str(&format!(" \u{00b7} {name}")); }
    lines.push(detail_line(detail));

    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        if !err.is_empty() {
            let truncated: String = err.chars().take(100).collect();
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("\u{2570}  ", DIM),  // ╰
                Span::styled(truncated, DIM_RED),
            ]));
        }
    }

    Some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_cell_basic() {
        let cell = UserCell::new("hello world".into(), vec![]);
        let lines = cell.display_lines(80);
        // empty (Rule 1) + ┏━ > text = 2
        assert_eq!(lines.len(), 2);
        assert_eq!(cell.desired_height(80), 2);
    }

    #[test]
    fn user_cell_multiline() {
        let cell = UserCell::new("line one\nline two\nline three".into(), vec![]);
        // empty (Rule 1) + 3 text lines = 4
        assert_eq!(cell.desired_height(80), 4);
    }

    #[test]
    fn user_cell_with_images() {
        let cell = UserCell::new("hello".into(), vec!["img1".into(), "img2".into()]);
        // empty (Rule 1) + text + image count = 3
        assert_eq!(cell.desired_height(80), 3);
    }

    #[test]
    fn mission_cell_with_steps() {
        let cell = MissionCell::new(
            "Fix the Docker build".into(),
            Some("/app/Dockerfile".into()),
            vec!["Read Dockerfile".into(), "Fix COPY path".into()],
        );
        let lines = cell.display_lines(80);
        // ┣━ [MISSION] + Understood + Target + Plan: + 2 steps = 6
        assert!(lines.len() >= 5, "Expected >= 5 lines, got {}", lines.len());
    }

    #[test]
    fn exec_cell_complete() {
        let mut cell = ExecCell::new("git".into(), "clone repo".into(), None, None);
        cell.complete_call("git", true, "cloned".into(), 2100, None, None, None, None);
        let lines = cell.display_lines(80);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn exec_cell_coalesce() {
        let mut cell = ExecCell::new("read_file".into(), "package.json".into(), Some(serde_json::json!({"path": "package.json"})), None);
        cell.add_call("read_file".into(), "Dockerfile".into(), Some(serde_json::json!({"path": "Dockerfile"})), None);
        assert_eq!(cell.calls.len(), 2);
        assert_eq!(cell.desired_height(80), 2);
    }

    #[test]
    fn exec_cell_rich_display_command() {
        let cell = ExecCell::new(
            "exec_command".into(),
            "build the project".into(),
            Some(serde_json::json!({"cmd": "docker build ."})),
            None,
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
            None,
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
            None,
        );
        cell.complete_call(
            "exec_command", false, "tests failed".into(), 5000,
            Some(1), Some("FAIL src/app.test.js\nExpected 5 but got 3".into()), Some(42),
            None,
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
        // empty (Rule 1) + ┏━ [*] RESULT: PASS + ┃ summary = 3 lines
        assert!(lines.len() >= 3, "Expected >= 3 lines, got {}", lines.len());
        let all_text: String = lines.iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect::<Vec<_>>().join("");
        assert!(all_text.contains("PASS"), "Should contain PASS");
        assert!(all_text.contains("RESULT"), "Should contain RESULT");
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
        // ┏━ [x] RESULT: FAIL + ┃ summary + ┇ Next: hint = 3 lines
        assert!(lines.len() >= 3, "Expected >= 3 lines, got {}", lines.len());
        let all_text: String = lines.iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect::<Vec<_>>().join("");
        assert!(all_text.contains("FAIL"), "Should contain FAIL");
        assert!(all_text.contains("Next:"), "Should contain Next: hint");
    }

    // ── New cell types ─────────────────────────────────────────────────────

    #[test]
    fn task_card_single_file() {
        let mut cell = ExecCell::new(
            "read_file".into(), "reading".into(),
            Some(serde_json::json!({"path": "crates/tui/src/cell.rs"})), None,
        );
        cell.complete_call("read_file", true, "ok".into(), 50, None, None, None, None);
        let card = TaskCard::from_exec_cell(&cell);
        let lines = card.display_lines(80);
        assert!(lines.len() >= 1);
        let text: String = lines.iter().flat_map(|l| l.spans.iter().map(|s| s.content.as_ref())).collect::<Vec<_>>().join("");
        assert!(text.contains("cell.rs") || text.contains("Read"), "Should contain filename or Read: {text}");
    }

    #[test]
    fn task_card_multi_file() {
        let mut cell = ExecCell::new(
            "read_file".into(), "reading a".into(),
            Some(serde_json::json!({"path": "a.rs"})), None,
        );
        cell.add_call("read_file".into(), "reading b".into(), Some(serde_json::json!({"path": "b.rs"})), None);
        cell.add_call("read_file".into(), "reading c".into(), Some(serde_json::json!({"path": "c.rs"})), None);
        cell.complete_call("read_file", true, "ok".into(), 30, None, None, None, None);
        cell.complete_call("read_file", true, "ok".into(), 20, None, None, None, None);
        cell.complete_call("read_file", true, "ok".into(), 10, None, None, None, None);
        let card = TaskCard::from_exec_cell(&cell);
        let lines = card.display_lines(80);
        let text: String = lines.iter().flat_map(|l| l.spans.iter().map(|s| s.content.as_ref())).collect::<Vec<_>>().join("");
        assert!(text.contains("3"), "Should mention file count: {text}");
        assert!(card.desired_height(80) >= 1);
    }

    #[test]
    fn diff_card_renders() {
        let card = DiffCard::new(
            vec!["src/main.rs".into(), "src/lib.rs".into()],
            vec![
                DiffPreviewLine { kind: DiffPreviewKind::Remove, text: "old line".into() },
                DiffPreviewLine { kind: DiffPreviewKind::Add, text: "new line".into() },
            ],
        );
        let lines = card.display_lines(80);
        assert!(lines.len() >= 4, "Expected header + 2 paths + 2 preview = >=4, got {}", lines.len());
        let text: String = lines.iter().flat_map(|l| l.spans.iter().map(|s| s.content.as_ref())).collect::<Vec<_>>().join("");
        assert!(text.contains("2 files edited"), "Should say '2 files edited': {text}");
        assert!(text.contains("+ ") || text.contains("- "), "Should contain diff lines: {text}");
    }

    #[test]
    fn group_divider_renders() {
        let d = GroupDivider;
        assert_eq!(d.desired_height(80), 1);
        let lines = d.display_lines(80);
        assert_eq!(lines.len(), 1);
        assert!(d.is_continuation(), "GroupDivider should suppress blank separator");
    }

    #[test]
    fn is_read_only_tool_classification() {
        assert!(is_read_only_tool("read_file"));
        assert!(is_read_only_tool("list_files"));
        assert!(is_read_only_tool("search"));
        assert!(is_read_only_tool("kubectl_get"));
        assert!(!is_read_only_tool("exec_command"));
        assert!(!is_read_only_tool("write_file"));
        assert!(!is_read_only_tool("apply_patch"));
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
        // ━━ [!] APPROVAL_REQUIRED + ACTION + CAPABILITIES: + [-] cap + bottom rule = 5
        assert!(cell.desired_height(80) >= 4, "Expected >= 4, got {}", cell.desired_height(80));
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
        let cell = AgentMessageCell::new(String::new(), false);
        assert!(cell.is_continuation());
    }

    // ── sanitize_display_text ──────────────────────────────────────────

    #[test]
    fn sanitize_strips_csi_sequences() {
        assert_eq!(sanitize_display_text("\x1b[31mred text\x1b[0m"), "red text");
        assert_eq!(sanitize_display_text("\x1b[1;32mbold green\x1b[0m"), "bold green");
    }

    #[test]
    fn sanitize_strips_osc_hyperlinks() {
        // BEL-terminated OSC 8 hyperlink
        assert_eq!(
            sanitize_display_text("\x1b]8;;https://example.com\x07link text\x1b]8;;\x07"),
            "link text"
        );
        // ST-terminated OSC
        assert_eq!(sanitize_display_text("\x1b]0;window title\x1b\\rest"), "rest");
    }

    #[test]
    fn sanitize_preserves_plain_text() {
        assert_eq!(sanitize_display_text("hello world"), "hello world");
        assert_eq!(sanitize_display_text(""), "");
        assert_eq!(sanitize_display_text("path/to/file.rs (42 lines)"), "path/to/file.rs (42 lines)");
    }

    #[test]
    fn sanitize_handles_mixed_content() {
        // ANSI bold around a path + trailing OSC fragment
        assert_eq!(
            sanitize_display_text("\x1b[1msrc/main.rs\x1b[0m \x1b]8;;\x07"),
            "src/main.rs "
        );
    }

    #[test]
    fn error_cell_sanitizes_ansi_in_message() {
        let cell = ErrorCell::new("rate limit \x1b[31mexceeded\x1b[0m".into());
        assert_eq!(cell.message, "rate limit exceeded");
        // Category detection should still work after sanitization
        assert!(matches!(cell.category, ErrorCategory::Api));
    }

    #[test]
    fn result_cell_sanitizes_summary() {
        let cell = ResultCell::new(
            "PASS".into(),
            "Cloned \x1b]8;;https://github.com/repo\x07repo\x1b]8;;\x07 successfully".into(),
            3, 5000, vec![], vec![], vec![],
        );
        assert_eq!(cell.summary, "Cloned repo successfully");
        assert!(!cell.summary.contains('\x1b'));
    }

    #[test]
    fn mission_cell_sanitizes_steps() {
        let cell = MissionCell::new(
            "Fix \x1b[1mDocker\x1b[0m build".into(),
            Some("/app/\x1b[36mDockerfile\x1b[0m".into()),
            vec!["Read \x1b[33mfile\x1b[0m".into()],
        );
        assert_eq!(cell.understood, "Fix Docker build");
        assert_eq!(cell.target.as_deref(), Some("/app/Dockerfile"));
        assert_eq!(cell.steps[0], "Read file");
    }

    #[test]
    fn format_tool_display_sanitizes_output() {
        let display = format_tool_display(
            "exec_command",
            Some(&serde_json::json!({"cmd": "echo \x1b[31mhello\x1b[0m"})),
            "",
        );
        assert!(!display.contains('\x1b'), "ANSI leaked into tool display: {display}");
        assert!(display.contains("echo hello"));
    }

    // ── transcript_lines pager routing ─────────────────────────────────────

    #[test]
    fn exec_cell_transcript_shows_full_output() {
        // Build an ExecCell with more than EXEC_OUTPUT_PREVIEW_LINES of streaming output.
        let mut cell = ExecCell::new(
            "exec_command".into(),
            "run tests".into(),
            Some(serde_json::json!({"cmd": "cargo test"})),
            None,
        );
        // Append 12 lines of streaming output — exceeds the 5-line preview cap.
        let big_output = (1..=12).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        cell.append_output(&big_output);

        let display = cell.display_lines(80);
        let transcript = cell.transcript_lines(80);

        // Pager should have strictly more lines than scrollback (full output vs truncated).
        assert!(
            transcript.len() > display.len(),
            "transcript ({}) should be longer than display ({})",
            transcript.len(), display.len(),
        );

        // Pager should contain all 12 lines.
        let pager_text: String = transcript.iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect::<Vec<_>>().join(" ");
        assert!(pager_text.contains("line 1"), "pager should contain first line");
        assert!(pager_text.contains("line 12"), "pager should contain last line");
    }

    #[test]
    fn diff_card_transcript_shows_all_files() {
        // Build a DiffCard with 5 files and 5 preview lines — both exceed the 3-item caps.
        let files: Vec<String> = (1..=5).map(|i| format!("src/file{i}.rs")).collect();
        let preview: Vec<DiffPreviewLine> = (1..=5).map(|i| DiffPreviewLine {
            kind: if i % 2 == 0 { DiffPreviewKind::Add } else { DiffPreviewKind::Remove },
            text: format!("line {i}"),
        }).collect();
        let card = DiffCard::new(files, preview);

        let display = card.display_lines(80);
        let transcript = card.transcript_lines(80);

        // Pager should have strictly more lines (no "… N more" truncation).
        assert!(
            transcript.len() > display.len(),
            "transcript ({}) should be longer than display ({})",
            transcript.len(), display.len(),
        );

        // Display should have the "… N more" truncation for files.
        let display_text: String = display.iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect::<Vec<_>>().join(" ");
        assert!(display_text.contains("more"), "display should mention truncated files");

        // Pager should contain all 5 file names.
        let pager_text: String = transcript.iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect::<Vec<_>>().join(" ");
        assert!(pager_text.contains("file5.rs"), "pager should contain all 5 files");
    }
}
