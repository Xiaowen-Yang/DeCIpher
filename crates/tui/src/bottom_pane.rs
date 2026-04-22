//! BottomPane — the fixed viewport region at the bottom of the terminal.
//!
//! Renders the input area, spinner, footer hints, command popup,
//! file search popup, streaming preview, and shortcut overlay.
//!
//! This replaces the manual `draw_prompt_inner()` function. ratatui handles
//! buffer diffing — no manual cursor tracking needed.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::app::{App, InputMode};
// shimmer removed — activity bar now uses a clean blinking dot (Claude Code style).

const DIM: Style = Style::new().add_modifier(Modifier::DIM);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
const CYAN: Style = Style::new().fg(Color::Cyan);
const BOLD_CYAN: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const YELLOW: Style = Style::new().fg(Color::Rgb(232, 163, 23));
const BOLD_YELLOW: Style = Style::new().fg(Color::Rgb(232, 163, 23)).add_modifier(Modifier::BOLD);
const GREEN: Style = Style::new().fg(Color::Green);
const RED: Style = Style::new().fg(Color::Red);

/// Gear spinner frames for the activity indicator (Industrial ASCII style).
/// Cycles through |, /, -, \ at 100ms per frame.
const GEAR_FRAMES: [char; 4] = ['|', '/', '-', '\\'];

/// Viewport height — the terminal is created ONCE with this height.
///
/// Kept small so the startup banner remains visible in terminals as short as
/// ~15 rows (matching Claude Code's behaviour).  Overlays that exceed this
/// height are truncated rather than pushing the banner off-screen.
///
/// Layout budget (normal): spinner (1) + divider (1) + input (1) + divider (1)
///                         + hints (1) + margin (1) = 6 rows.
pub const MAX_PANE_HEIGHT: u16 = 6;

/// The bottom pane widget — renders the prompt region.
///
/// This is the viewport portion managed by `terminal.draw()`.
/// It builds up all lines, renders them into the buffer, and
/// reports the cursor position for the input field.
pub struct BottomPane<'a> {
    pub app: &'a App,
}

impl<'a> BottomPane<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Build all the lines for the bottom pane.
    /// Build actual content lines (variable height based on current UI state).
    fn build_content_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Command popup
        if self.app.mode == InputMode::CommandPopup {
            self.build_command_popup(&mut lines);
        }

        // File search popup
        if self.app.mode == InputMode::FileSearch && !self.app.file_search_results.is_empty() {
            self.build_file_search_popup(&mut lines);
        }

        // Streaming delta preview (partial line not yet committed).
        // Always shown during streaming to anchor the blinking cursor.
        if self.app.chat.is_streaming() {
            let partial = self.app.chat.partial_line();
            // Blink cursor: on/off every 8 spinner frames (~256ms at 32ms/frame).
            let cursor = if (self.app.spinner_frame / 8) % 2 == 0 { "\u{258c}" } else { " " };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::raw(partial.to_string()),
                Span::styled(cursor, BOLD_CYAN),
            ]));
        }

        // Live activity bar — shown whenever the agent is busy or waiting for approval.
        // This replaces the legacy spinner_label gate: the bar is driven by agent_phase,
        // not by whether the Node.js server has sent a Spinner message.
        let show_activity_bar = self.app.agent_busy
            || self.app.mode == InputMode::ApprovalPending
            || self.app.agent_phase.is_active();
        if show_activity_bar {
            let label = if let Some(ref lbl) = self.app.spinner_label {
                lbl.as_str()
            } else {
                self.app.agent_phase.label()
            };
            self.build_spinner(label, &mut lines);
        }

        // Approval popup — visible, interactive overlay with arrow-key selection.
        if self.app.mode == InputMode::ApprovalPending {
            self.build_approval_popup(&mut lines);
        }

        // Queued message indicator
        if self.app.queued_message.is_some() {
            lines.push(Line::from(vec![
                Span::styled("\u{2507}  ", DIM),  // ┇
                Span::styled("[WAIT] ", YELLOW),
                Span::styled("Message queued -- will send when agent finishes", DIM),
            ]));
        }

        // Input line(s) — pinned at the bottom
        self.build_input_lines(&mut lines);

        // Footer hints
        self.build_hints(&mut lines);

        // Shortcut overlay
        if self.app.show_shortcuts {
            self.build_shortcuts(&mut lines);
        }

        lines
    }

    /// Build the full viewport lines: blank top-padding + content.
    ///
    /// Content is truncated (from the top) to fit MAX_PANE_HEIGHT so that
    /// overlays like shortcuts don't push the viewport beyond its budget.
    /// Blank padding pins the content to the viewport bottom.
    fn build_lines(&self) -> Vec<Line<'static>> {
        let mut content = self.build_content_lines();
        let max = MAX_PANE_HEIGHT as usize;
        // Truncate excess content from the top (keep the bottom — input + hints).
        if content.len() > max {
            let skip = content.len() - max;
            content = content.into_iter().skip(skip).collect();
        }
        let pad = max.saturating_sub(content.len());
        let mut lines = Vec::with_capacity(max);
        for _ in 0..pad {
            lines.push(Line::from(""));
        }
        lines.extend(content);
        lines
    }

    fn build_command_popup(&self, lines: &mut Vec<Line<'static>>) {
        let filtered = self.app.filtered_commands();
        let max_show = filtered.len().min(10);
        for (i, cmd) in filtered.iter().take(max_show).enumerate() {
            let (name_style, desc_style) = if i == self.app.popup_index {
                (BOLD_CYAN, Style::default().fg(Color::White))
            } else {
                (CYAN, DIM)
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:<14}", cmd.name), name_style),
                Span::styled(cmd.description.clone(), desc_style),
            ]));
        }
        if filtered.len() > max_show {
            lines.push(Line::from(Span::styled(
                format!("  \u{2026} {} more", filtered.len() - max_show),
                DIM,
            )));
        }
    }

    fn build_approval_popup(&self, lines: &mut Vec<Line<'static>>) {
        // Tool action being requested.
        if let Some(ref action) = self.app.pending_approval_action {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(action.clone(), BOLD_YELLOW),
            ]));
        }
        // Selectable options with > prefix and [y]/[a]/[n] switches.
        let options = [
            ("y", "Approve", "allow this action"),
            ("a", "Always", "approve all future actions"),
            ("n", "Deny", "reject and stop"),
        ];
        for (i, (key, label, desc)) in options.iter().enumerate() {
            let selected = i == self.app.approval_index;
            let (marker, key_style, label_style, desc_style) = if selected {
                ("> ", BOLD_CYAN, BOLD_CYAN, Style::default().fg(Color::White))
            } else {
                ("  ", DIM, DIM, DIM)
            };
            lines.push(Line::from(vec![
                Span::styled(marker, if selected { BOLD_CYAN } else { DIM }),
                Span::styled(format!("[{key}] "), key_style),
                Span::styled(format!("{label:<8}"), label_style),
                Span::styled(desc.to_string(), desc_style),
            ]));
        }
    }

    fn build_file_search_popup(&self, lines: &mut Vec<Line<'static>>) {
        let max_show = self.app.file_search_results.len().min(10);
        for (i, result) in self.app.file_search_results.iter().take(max_show).enumerate() {
            let icon = if result.is_dir {
                "\u{1f4c1}"
            } else if result.is_image {
                "\u{1f5bc}"
            } else {
                "  "
            };
            let (icon_style, path_style) = if i == self.app.file_search_index {
                (BOLD_CYAN, BOLD_CYAN)
            } else {
                (DIM, DIM)
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(icon.to_string(), icon_style),
                Span::raw(" "),
                Span::styled(result.path.clone(), path_style),
            ]));
        }
        if self.app.file_search_results.len() > max_show {
            lines.push(Line::from(Span::styled(
                format!("  \u{2026} {} more", self.app.file_search_results.len() - max_show),
                DIM,
            )));
        }
    }

    fn build_input_lines(&self, lines: &mut Vec<Line<'static>>) {
        let w = self.app.chat.width() as usize;
        let rule = "\u{2501}".repeat(w); // ━━━━━━...  heavy horizontal rule

        // Top rule
        lines.push(Line::from(Span::styled(rule.clone(), DIM)));

        // Content lines: "  > text" (no side border)
        // [Image #N] tokens are highlighted in cyan.
        let input_lines: Vec<&str> = self.app.input.split('\n').collect();
        for (i, line) in input_lines.iter().enumerate() {
            let mut spans: Vec<Span<'static>> = if i == 0 {
                vec![Span::styled("  ", DIM), Span::styled("> ", YELLOW)]
            } else {
                vec![Span::styled("    ", DIM)]
            };
            let mut rest = *line;
            while let Some(start) = rest.find("[Image #") {
                if start > 0 {
                    spans.push(Span::styled(rest[..start].to_string(), BOLD));
                }
                if let Some(end) = rest[start..].find(']') {
                    let token = &rest[start..start + end + 1];
                    spans.push(Span::styled(token.to_string(), BOLD_CYAN));
                    rest = &rest[start + end + 1..];
                } else {
                    break;
                }
            }
            if !rest.is_empty() {
                spans.push(Span::styled(rest.to_string(), BOLD));
            }
            lines.push(Line::from(spans));
        }

        // Bottom rule
        lines.push(Line::from(Span::styled(rule, DIM)));
    }

    fn build_spinner(&self, label: &str, lines: &mut Vec<Line<'static>>) {
        // Gear spinner: cycles |, /, -, \ (Industrial ASCII style).
        let frame_idx = self.app.spinner_started
            .map(|t| (t.elapsed().as_millis() / 100) as usize % 4)
            .unwrap_or(0);
        let gear = GEAR_FRAMES[frame_idx];

        let phase = &self.app.agent_phase;

        // Phase label: spec vocabulary takes precedence over raw spinner label.
        let display_label = if phase.is_active() {
            phase.label()
        } else {
            label
        };

        let is_approval = *phase == crate::app::AgentPhase::WaitingForApproval;
        let gear_style = if is_approval {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::Cyan)
        };

        let mut spans = vec![
            Span::styled(format!("{gear}"), gear_style),
            Span::raw(" "),
            Span::styled(display_label.to_string(), Style::default().fg(Color::White)),
        ];

        // Inline detail: > separator + context
        if let Some(ref detail) = self.app.agent_phase_detail {
            let truncated: String = detail.chars().take(40).collect();
            spans.push(Span::styled(format!(" > {truncated}"), DIM));
        }

        // Turn counter
        if self.app.agent_turn > 0 {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("[turn {}]", self.app.agent_turn),
                DIM,
            ));
        }

        // Model name
        if let Some(ref banner) = self.app.banner {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(banner.model.clone(), DIM));
        }

        // Elapsed time
        if let Some(started) = self.app.mission_started {
            let secs = started.elapsed().as_secs();
            let display = if secs >= 60 {
                format!("{}m {}s", secs / 60, secs % 60)
            } else {
                format!("{secs}s")
            };
            spans.push(Span::raw("  "));
            spans.push(Span::styled(display, DIM));
        }

        // Cumulative token count
        if self.app.total_tokens > 0 {
            let display = if self.app.total_tokens >= 1_000_000 {
                format!("{:.1}M tok", self.app.total_tokens as f64 / 1_000_000.0)
            } else if self.app.total_tokens >= 1_000 {
                format!("{:.1}K tok", self.app.total_tokens as f64 / 1_000.0)
            } else {
                format!("{} tok", self.app.total_tokens)
            };
            spans.push(Span::raw("  "));
            spans.push(Span::styled(display, DIM));
        }

        // Right-side hint
        if is_approval {
            spans.push(Span::styled("  y/n to decide", YELLOW));
        } else {
            spans.push(Span::styled("  esc to interrupt", DIM));
        }

        lines.push(Line::from(spans));
    }

    fn build_hints(&self, lines: &mut Vec<Line<'static>>) {
        // Left-side hint text.
        let left_text: String = match self.app.mode {
            InputMode::ApprovalPending => "  \u{2191}\u{2193} select  Enter confirm  y/a/n direct".to_string(),
            InputMode::CommandPopup => "  \u{2191}\u{2193} navigate  Enter select  Esc cancel".to_string(),
            InputMode::HistorySearch => {
                let status = if self.app.search_match_index.is_some() {
                    "match"
                } else if self.app.search_query.is_empty() {
                    ""
                } else {
                    "no match"
                };
                format!(
                    "  (reverse-i-search)`{}': {}  Ctrl+R older  Enter accept  Esc cancel",
                    self.app.search_query, status
                )
            }
            InputMode::FileSearch => "  \u{2191}\u{2193} navigate  Tab/Enter select  Esc cancel".to_string(),
            InputMode::Pager => "  j/k scroll  q quit".to_string(),
            InputMode::Normal => {
                // Show "ctrl+c again to quit" if Ctrl+C was pressed within the last 2s.
                let ctrl_c_primed = self.app.last_ctrl_c
                    .map(|t| t.elapsed() < std::time::Duration::from_secs(2))
                    .unwrap_or(false);
                if ctrl_c_primed && !self.app.agent_busy {
                    "  ctrl+c again to quit".to_string()
                } else if self.app.agent_busy {
                    "  Ctrl+C interrupt".to_string()
                } else {
                    "  Enter submit  / commands  Ctrl+R search  ? shortcuts".to_string()
                }
            }
        };

        // Right-side: context budget display with usage bar.
        // Shows prompt tokens / context window with a visual budget indicator.
        let right_spans: Vec<Span<'static>> = match self.app.mode {
            InputMode::Normal | InputMode::Pager if self.app.context_tokens > 0 => {
                let ctx = self.app.context_tokens;
                let cw = self.app.context_window;
                if cw > 0 {
                    let pct = (ctx as f64 / cw as f64 * 100.0).min(100.0);
                    let bar_width: usize = 8;
                    let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
                    let empty = bar_width.saturating_sub(filled);
                    let bar_color = if pct > 80.0 {
                        Style::default().fg(Color::Red)
                    } else if pct > 60.0 {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default().fg(Color::Green)
                    };
                    vec![
                        Span::styled(
                            format!("{}/{} ", format_tokens(ctx), format_tokens(cw)),
                            DIM,
                        ),
                        Span::styled("\u{2588}".repeat(filled), bar_color),
                        Span::styled("\u{2591}".repeat(empty), DIM),
                        Span::styled("  ", DIM),
                    ]
                } else {
                    vec![Span::styled(format!("{}  ", format_tokens(ctx)), DIM)]
                }
            }
            _ => Vec::new(),
        };

        let total_width = self.app.chat.width() as usize;
        let left_len = left_text.len();
        let right_len: usize = right_spans.iter().map(|s| s.content.len()).sum();

        if right_len > 0 && left_len + right_len + 2 < total_width {
            let pad = total_width.saturating_sub(left_len + right_len);
            let mut spans = vec![
                Span::styled(left_text, DIM),
                Span::raw(" ".repeat(pad)),
            ];
            spans.extend(right_spans);
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(Span::styled(left_text, DIM)));
        }
    }

    fn build_shortcuts(&self, lines: &mut Vec<Line<'static>>) {
        let shortcuts = [
            ("Ctrl+A/E", "start/end of line"),
            ("Ctrl+K/U", "kill to end/start"),
            ("Ctrl+W", "kill word backward"),
            ("Alt+D", "kill word forward"),
            ("Ctrl+Y", "yank (paste kill buffer)"),
            ("Ctrl+B/F", "move left/right"),
            ("Alt+B/F", "move word left/right"),
            ("Ctrl+R", "reverse history search"),
            ("Ctrl+T", "transcript pager"),
            ("Ctrl+V", "paste image from clipboard"),
            ("Ctrl+X", "open $EDITOR"),
            ("Ctrl+Z", "suspend (unix)"),
            ("@", "file search popup"),
            ("Shift+Enter", "insert newline"),
            ("Tab", "submit / queue if busy"),
            ("/ (empty)", "open command popup"),
            ("? (empty)", "toggle this overlay"),
        ];
        lines.push(Line::from(Span::styled(
            "\u{250f}\u{2501}\u{2501} SHORTCUTS \u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}",
            DIM,
        )));
        for (key, desc) in &shortcuts {
            lines.push(Line::from(vec![
                Span::styled("\u{2503}  ", DIM),  // ┃
                Span::styled(format!("{:<16}", key), CYAN),
                Span::styled(*desc, DIM),
            ]));
        }
        lines.push(Line::from(Span::styled(
            "\u{2517}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}",
            DIM,
        )));
    }

    /// Fixed viewport height — always MAX_PANE_HEIGHT, never changes.
    pub fn desired_height(&self) -> u16 {
        MAX_PANE_HEIGHT
    }

    /// Compute cursor position (x, y) within the viewport area.
    pub fn cursor_position(&self, area: Rect) -> Option<(u16, u16)> {
        if self.app.mode == InputMode::Pager {
            return None; // No cursor in pager mode
        }

        // Use the same build_lines() output to find the cursor row,
        // ensuring truncation and padding are accounted for identically.
        let lines = self.build_lines();

        // The input line is the one starting with "> " prompt.
        // Find its index from the bottom (it's always near the end,
        // followed by bottom-rule + hints + optional shortcuts).
        let mut input_row: Option<u16> = None;
        for (i, line) in lines.iter().enumerate() {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if text.contains("> ") && !text.contains("[>") {
                input_row = Some(i as u16);
                break;
            }
        }
        let input_row_start = input_row.unwrap_or(1); // fallback

        // Find cursor position within input
        let (cursor_line, cursor_col) = cursor_position_in_multiline(&self.app.input, self.app.cursor);
        let y = area.y + input_row_start + cursor_line as u16;
        let x = area.x + cursor_col as u16 + 4; // 4 = "│ ❯ " prefix width

        if y < area.y + MAX_PANE_HEIGHT {
            Some((x.min(area.x + area.width - 1), y))
        } else {
            None
        }
    }
}

impl Widget for BottomPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let lines = self.build_lines();

        for (i, line) in lines.iter().enumerate() {
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }
            let line_area = Rect::new(area.x, y, area.width, 1);
            buf.set_line(line_area.x, line_area.y, line, line_area.width);
        }
    }
}

fn cursor_position_in_multiline(text: &str, cursor: usize) -> (usize, usize) {
    let mut line = 0;
    let mut col = 0;
    for (i, ch) in text.char_indices() {
        if i >= cursor {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}


fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

// ── Scrollback rendering helpers ───────────────────────────────────────────

/// Render banner as Lines for insert_before.
pub fn banner_lines(
    version: &str,
    provider: &str,
    model: &str,
    directory: &str,
    api_key_set: bool,
    instructions_display: Option<&str>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // ┏━ DeCIpher v0.2.0 [SYS_READY]
    lines.push(Line::from(vec![
        Span::styled("\u{250f}\u{2501} ", DIM),  // ┏━
        Span::styled("DeCIpher", BOLD_CYAN),
        Span::raw(" "),
        Span::styled(format!("v{}", version), DIM),
        Span::raw(" "),
        Span::styled("[SYS_READY]", DIM),
    ]));
    // Mascot + config matrix
    lines.push(Line::from(vec![
        Span::styled("\u{2503}    ", DIM),  // ┃
        Span::styled("/\\_/\\", BOLD_YELLOW),
        Span::styled("     [PRV] ", DIM),
        Span::styled(provider.to_string(), CYAN),
    ]));
    lines.push(Line::from(vec![
        Span::styled("\u{2503}   ", DIM),  // ┃
        Span::styled("( \u{2022}\u{1d25}\u{2022} )", BOLD_YELLOW),
        Span::styled("    [MOD] ", DIM),
        Span::styled(model.to_string(), CYAN),
    ]));
    lines.push(Line::from(vec![
        Span::styled("\u{2503}              ", DIM),  // ┃
        Span::styled("[DIR] ", DIM),
        Span::styled(directory.to_string(), Style::default().fg(Color::White)),
    ]));
    if let Some(instr) = instructions_display {
        lines.push(Line::from(vec![
            Span::styled("\u{2503}              ", DIM),  // ┃
            Span::styled("[INS] ", DIM),
            Span::styled(instr.to_string(), CYAN),
        ]));
    }
    let api_switch = if api_key_set {
        Span::styled("[*]", GREEN)
    } else {
        Span::styled("[x]", RED)
    };
    lines.push(Line::from(vec![
        Span::styled("\u{2503}   ", DIM),  // ┃
        api_switch,
        Span::styled(" API    ", DIM),
        Span::styled("[AUT] ", DIM),
        Span::styled("on-request", CYAN),
    ]));
    lines.push(Line::from(Span::styled("\u{2503}", DIM)));
    lines.push(Line::from(vec![
        Span::styled("\u{2503}  ", DIM),  // ┃
        Span::styled("Type a mission, paste a path, or ", DIM),
        Span::styled("/help", CYAN),
        Span::styled(" for commands.", DIM),
    ]));
    lines.push(Line::from(vec![
        Span::styled("\u{2503}  ", DIM),  // ┃
        Span::styled("ctrl+r", DIM),
        Span::styled(" history  ", DIM),
        Span::styled("ctrl+t", DIM),
        Span::styled(" transcript  ", DIM),
        Span::styled("ctrl+c", DIM),
        Span::styled(" quit", DIM),
    ]));
    lines.push(Line::from(""));
    lines
}

/// Render user input as Lines for insert_before.
///
/// `[Image #N]` tokens are highlighted in cyan to match Claude Code style.
pub fn user_input_lines(text: &str) -> Vec<Line<'static>> {
    let input_lines: Vec<&str> = text.split('\n').collect();
    let mut lines = Vec::with_capacity(input_lines.len() + 1);
    for (i, line) in input_lines.iter().enumerate() {
        let prefix: Vec<Span<'static>> = if i == 0 {
            // ┏━ > user text
            vec![
                Span::styled("\u{250f}\u{2501} ", DIM),  // ┏━
                Span::styled("> ", YELLOW),
            ]
        } else {
            // ┃ continuation
            vec![Span::styled("\u{2503} ", DIM)]
        };
        let mut spans = prefix;
        // Split on [Image #N] tokens and style them distinctly.
        let mut rest = *line;
        while let Some(start) = rest.find("[Image #") {
            if start > 0 {
                spans.push(Span::raw(rest[..start].to_string()));
            }
            if let Some(end) = rest[start..].find(']') {
                let token = &rest[start..start + end + 1];
                spans.push(Span::styled(token.to_string(), BOLD_CYAN));
                rest = &rest[start + end + 1..];
            } else {
                break;
            }
        }
        if !rest.is_empty() {
            spans.push(Span::raw(rest.to_string()));
        }
        lines.push(Line::from(spans));
    }
    lines
}

// server_message_lines() — removed in Phase 3.
// Cell creation and scrollback rendering is now handled by ChatWidget::handle_server_message().

/// Goodbye message for insert_before.
pub fn goodbye_lines() -> Vec<Line<'static>> {
    vec![Line::from(Span::styled("  Goodbye!", DIM))]
}

/// Render Lines into a Buffer (for insert_before).
///
/// Unlike `buf.set_line()` which pads every line with styled spaces to the
/// full buffer width (locking lines to a fixed width in terminal scrollback),
/// this writes only actual content characters. Remaining cells stay at the
/// default (unstyled space), which ratatui's diff engine skips when rendering.
/// This allows the terminal emulator to re-wrap content naturally on resize.
pub fn render_lines_to_buffer(buf: &mut Buffer, lines: &[Line<'_>]) {
    let area = buf.area;
    for (i, line) in lines.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }
        // Write only the actual span content — no trailing space padding.
        // This lets the terminal emulator re-wrap text on resize.
        let mut x = area.x;
        for span in &line.spans {
            for ch in span.content.chars() {
                if x >= area.x + area.width {
                    break;
                }
                let cell = &mut buf[(x, y)];
                cell.set_char(ch);
                cell.set_style(span.style);
                x += 1;
            }
        }
        // Remaining cells stay at buffer default (empty space, no style).
        // ratatui's diff engine won't write these to the terminal, so
        // the terminal emulator sees only the actual text width.
    }
}
