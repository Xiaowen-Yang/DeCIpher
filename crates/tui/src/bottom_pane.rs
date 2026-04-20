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
use crate::shimmer;

const DIM: Style = Style::new().add_modifier(Modifier::DIM);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
const CYAN: Style = Style::new().fg(Color::Cyan);
const BOLD_CYAN: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const YELLOW: Style = Style::new().fg(Color::Rgb(232, 163, 23));
const BOLD_YELLOW: Style = Style::new().fg(Color::Rgb(232, 163, 23)).add_modifier(Modifier::BOLD);
const GREEN: Style = Style::new().fg(Color::Green);
const RED: Style = Style::new().fg(Color::Red);

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

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
    fn build_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Command popup
        if self.app.mode == InputMode::CommandPopup {
            self.build_command_popup(&mut lines);
        }

        // File search popup
        if self.app.mode == InputMode::FileSearch && !self.app.file_search_results.is_empty() {
            self.build_file_search_popup(&mut lines);
        }

        // Streaming delta preview (partial line not yet committed)
        if self.app.stream.active && !self.app.stream.partial_line().is_empty() {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::raw(self.app.stream.partial_line().to_string()),
            ]));
        }

        // Input line(s)
        self.build_input_lines(&mut lines);

        // Spinner
        if let Some(ref label) = self.app.spinner_label {
            self.build_spinner(label, &mut lines);
        }

        // Queued message indicator
        if self.app.queued_message.is_some() {
            lines.push(Line::from(vec![
                Span::styled("  ", DIM),
                Span::styled("\u{23f3} ", YELLOW),
                Span::styled("Message queued \u{2014} will send when agent finishes", DIM),
            ]));
        }

        // Footer hints
        self.build_hints(&mut lines);

        // Shortcut overlay
        if self.app.show_shortcuts {
            self.build_shortcuts(&mut lines);
        }

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
        let input_lines: Vec<&str> = self.app.input.split('\n').collect();
        for (i, line) in input_lines.iter().enumerate() {
            if i == 0 {
                lines.push(Line::from(vec![
                    Span::styled("\u{2502} ", DIM),
                    Span::styled("\u{276f} ", BOLD_YELLOW),
                    Span::styled(line.to_string(), BOLD),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("\u{2502}   ", DIM),
                    Span::styled(line.to_string(), BOLD),
                ]));
            }
        }
    }

    fn build_spinner(&self, label: &str, lines: &mut Vec<Line<'static>>) {
        let frame_idx = self.app.spinner_started
            .map(|t| (t.elapsed().as_millis() / 80) as usize)
            .unwrap_or(self.app.spinner_frame);
        let frame = SPINNER_FRAMES[frame_idx % SPINNER_FRAMES.len()];
        let elapsed = self.app.spinner_started
            .map(|t| format!(" ({:.1}s)", t.elapsed().as_secs_f64()))
            .unwrap_or_default();

        // Build shimmer spans (convert crossterm Color → ratatui Color)
        let shimmer_chars = shimmer::shimmer_chars(label);
        let mut spans = vec![
            Span::raw("  "),
            Span::styled(frame.to_string(), CYAN),
            Span::raw(" "),
        ];
        for (ch, color) in &shimmer_chars {
            let rat_color = crossterm_to_ratatui_color(*color);
            spans.push(Span::styled(ch.to_string(), Style::default().fg(rat_color)));
        }
        spans.push(Span::styled(elapsed, DIM));

        lines.push(Line::from(spans));
    }

    fn build_hints(&self, lines: &mut Vec<Line<'static>>) {
        let hints = match self.app.mode {
            InputMode::ApprovalPending => "  y approve  a always  n deny  Esc cancel".to_string(),
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
                let token_info = if self.app.total_tokens > 0 {
                    format!("  [{} tokens]", format_tokens(self.app.total_tokens))
                } else {
                    String::new()
                };
                if self.app.agent_busy {
                    format!("  Ctrl+C interrupt{token_info}")
                } else {
                    format!("  Enter submit  / commands  Ctrl+R search  Ctrl+C quit{token_info}")
                }
            }
        };
        lines.push(Line::from(Span::styled(hints, DIM)));
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
            "  \u{250c} SHORTCUTS \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            DIM,
        )));
        for (key, desc) in &shortcuts {
            lines.push(Line::from(vec![
                Span::styled("  \u{2502} ", DIM),
                Span::styled(format!("{:<16}", key), CYAN),
                Span::styled(*desc, DIM),
            ]));
        }
        lines.push(Line::from(Span::styled(
            "  \u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            DIM,
        )));
    }

    /// Compute the height needed for this pane.
    pub fn desired_height(&self) -> u16 {
        self.build_lines().len() as u16
    }

    /// Compute cursor position (x, y) within the viewport area.
    pub fn cursor_position(&self, area: Rect) -> Option<(u16, u16)> {
        if self.app.mode == InputMode::Pager {
            return None; // No cursor in pager mode
        }

        let lines = self.build_lines();

        // Find the input line row — count lines before input
        let mut input_row_start = 0u16;

        // Count popup lines
        if self.app.mode == InputMode::CommandPopup {
            let filtered = self.app.filtered_commands();
            input_row_start += filtered.len().min(10) as u16;
            if filtered.len() > 10 { input_row_start += 1; }
        }
        if self.app.mode == InputMode::FileSearch && !self.app.file_search_results.is_empty() {
            let max_show = self.app.file_search_results.len().min(10);
            input_row_start += max_show as u16;
            if self.app.file_search_results.len() > max_show { input_row_start += 1; }
        }
        // Streaming preview
        if self.app.stream.active && !self.app.stream.partial_line().is_empty() {
            input_row_start += 1;
        }

        // Find cursor position within input
        let (cursor_line, cursor_col) = cursor_position_in_multiline(&self.app.input, self.app.cursor);
        let y = area.y + input_row_start + cursor_line as u16;
        let x = area.x + cursor_col as u16 + 4; // 4 = "│ ❯ " prefix width

        if y < area.y + lines.len() as u16 {
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

/// Convert crossterm Color to ratatui Color.
/// ratatui re-exports crossterm colors but they're different enum types.
fn crossterm_to_ratatui_color(c: crossterm::style::Color) -> Color {
    match c {
        crossterm::style::Color::Rgb { r, g, b } => Color::Rgb(r, g, b),
        crossterm::style::Color::Black => Color::Black,
        crossterm::style::Color::Red => Color::Red,
        crossterm::style::Color::Green => Color::Green,
        crossterm::style::Color::Yellow => Color::Yellow,
        crossterm::style::Color::Blue => Color::Blue,
        crossterm::style::Color::Magenta => Color::Magenta,
        crossterm::style::Color::Cyan => Color::Cyan,
        crossterm::style::Color::White => Color::White,
        crossterm::style::Color::DarkGrey => Color::DarkGray,
        crossterm::style::Color::Grey => Color::Gray,
        crossterm::style::Color::DarkRed => Color::Red,
        crossterm::style::Color::DarkGreen => Color::Green,
        crossterm::style::Color::DarkYellow => Color::Yellow,
        crossterm::style::Color::DarkBlue => Color::Blue,
        crossterm::style::Color::DarkMagenta => Color::Magenta,
        crossterm::style::Color::DarkCyan => Color::Cyan,
        crossterm::style::Color::AnsiValue(v) => Color::Indexed(v),
        _ => Color::Reset,
    }
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
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled("/\\_/\\", BOLD_YELLOW),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled("( \u{2022}\u{1d25}\u{2022} )", BOLD_YELLOW),
        Span::raw("  "),
        Span::styled("DeCIpher", BOLD_CYAN),
        Span::raw(" "),
        Span::styled(format!("v{}", version), DIM),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  provider   ", DIM),
        Span::styled(provider.to_string(), CYAN),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  model      ", DIM),
        Span::styled(model.to_string(), CYAN),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  directory  ", DIM),
        Span::styled(directory.to_string(), Style::default().fg(Color::White)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  approval   ", DIM),
        Span::styled("on-request", CYAN),
        Span::styled("  last: ", DIM),
        Span::styled("idle", DIM),
    ]));
    let api_key_line = if api_key_set {
        Line::from(vec![
            Span::styled("  api key    ", DIM),
            Span::styled("\u{25cf}", GREEN),
            Span::styled(" configured", DIM),
        ])
    } else {
        Line::from(vec![
            Span::styled("  api key    ", DIM),
            Span::styled("\u{25cb}", RED),
            Span::styled(" not set", RED),
        ])
    };
    lines.push(api_key_line);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Type a mission, paste a path, or ", DIM),
        Span::styled("/help", CYAN),
        Span::styled(" for commands.", DIM),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  ctrl+r", DIM),
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
pub fn user_input_lines(text: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled("\u{256d}\u{2500}", DIM)),
        Line::from(vec![
            Span::styled("\u{2502} ", DIM),
            Span::styled("\u{276f} ", BOLD_YELLOW),
            Span::styled(text.to_string(), Style::default()),
        ]),
        Line::from(Span::styled("\u{256e}\u{2500}", DIM)),
    ]
}

/// Render a server message as Lines for insert_before.
pub fn server_message_lines(msg: &decipher_protocol::ServerMessage, suppress_agent: &mut bool) -> Vec<Line<'static>> {
    use decipher_protocol::ServerMessage;

    match msg {
        ServerMessage::Banner { version, provider, model, directory, api_key_set } => {
            banner_lines(version, provider, model, directory, *api_key_set)
        }
        ServerMessage::Mission { understood, target, steps, .. } => {
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
                Span::styled(understood.clone(), Style::default().fg(Color::White)),
            ]));
            if let Some(t) = target {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("Target: ", DIM),
                    Span::styled(t.clone(), CYAN),
                ]));
            }
            if !steps.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("Plan:", DIM),
                ]));
                for (i, s) in steps.iter().enumerate() {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(format!("{}. ", i + 1), DIM),
                        Span::raw(s.clone()),
                    ]));
                }
            }
            lines.push(Line::from(""));
            lines
        }
        ServerMessage::Clarification { question } => {
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
                    Span::styled(question.clone(), Style::default().fg(Color::White)),
                ]),
                Line::from(Span::styled("  Reply below and DeCIpher will continue.", DIM)),
                Line::from(""),
            ]
        }
        ServerMessage::ApprovalRequest { action, capabilities } => {
            let mut lines = Vec::new();
            lines.push(Line::from(""));
            let bar = "\u{2500}".repeat(56);
            lines.push(Line::from(vec![
                Span::styled("  \u{250c}\u{2500} ", DIM),
                Span::styled("APPROVAL", BOLD_YELLOW),
                Span::styled(format!(" {}", bar), DIM),
            ]));
            lines.push(Line::from(Span::styled("  \u{2502}", DIM)));
            if let Some(act) = action {
                lines.push(Line::from(vec![
                    Span::styled("  \u{2502} ", DIM),
                    Span::styled("Action: ", BOLD),
                    Span::styled(act.tool.clone(), CYAN),
                ]));
                if let Some(reason) = &act.reasoning {
                    lines.push(Line::from(vec![
                        Span::styled("  \u{2502} ", DIM),
                        Span::styled(reason.clone(), DIM),
                    ]));
                }
                lines.push(Line::from(Span::styled("  \u{2502}", DIM)));
            }
            lines.push(Line::from(vec![
                Span::styled("  \u{2502} ", DIM),
                Span::raw("DeCIpher requests these capabilities:"),
            ]));
            for c in capabilities {
                lines.push(Line::from(vec![
                    Span::styled("  \u{2502}   ", DIM),
                    Span::styled("\u{203a} ", YELLOW),
                    Span::raw(c.clone()),
                ]));
            }
            lines.push(Line::from(Span::styled("  \u{2502}", DIM)));
            lines.push(Line::from(vec![
                Span::styled("  \u{2502}  ", DIM),
                Span::raw("Scope: this session only. Nothing pushed or deployed."),
            ]));
            lines.push(Line::from(Span::styled("  \u{2502}", DIM)));
            lines.push(Line::from(vec![
                Span::styled("  \u{2502}  ", DIM),
                Span::styled("y", BOLD),
                Span::styled(" approve  ", DIM),
                Span::styled("a", BOLD),
                Span::styled(" always  ", DIM),
                Span::styled("n", BOLD),
                Span::styled(" deny", DIM),
            ]));
            lines.push(Line::from(vec![
                Span::styled(format!("  \u{2514}{}\u{2500}", bar), DIM),
            ]));
            lines
        }
        ServerMessage::ToolStart { tool, reasoning } => {
            let r: String = reasoning.chars().take(60).collect();
            vec![Line::from(vec![
                Span::raw("  "),
                Span::styled(SPINNER_FRAMES[0], CYAN),
                Span::raw(" "),
                Span::styled(format!("{tool} \u{2014} {r}"), DIM),
            ])]
        }
        ServerMessage::ToolResult { tool, success, summary, elapsed_ms } => {
            let s = *elapsed_ms as f64 / 1000.0;
            let icon = if *success {
                Span::styled("\u{2713}", GREEN)
            } else {
                Span::styled("\u{2717}", RED)
            };
            vec![Line::from(vec![
                Span::raw("  "),
                icon,
                Span::raw(format!(" {tool} \u{2014} {summary} ")),
                Span::styled(format!("({s:.1}s)"), DIM),
            ])]
        }
        ServerMessage::AgentMessage { text } => {
            if *suppress_agent {
                *suppress_agent = false;
                return Vec::new();
            }
            // Simple text rendering — each line indented by 2 spaces
            text.lines()
                .map(|line| Line::from(format!("  {}", line)))
                .collect()
        }
        ServerMessage::MissionComplete { outcome, summary, turns, elapsed_ms, .. } => {
            let s = *elapsed_ms as f64 / 1000.0;
            let w = 60;
            let mut lines = Vec::new();
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("\u{2500}".repeat(w), DIM)));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  [RESULT]", BOLD)));
            let outcome_span = if outcome == "PASS" {
                Span::styled(format!("PASS ({s:.1}s)"), GREEN)
            } else {
                Span::styled(format!("FAIL ({s:.1}s)"), RED)
            };
            lines.push(Line::from(vec![
                Span::raw("  Outcome:     "),
                outcome_span,
            ]));
            lines.push(Line::from(format!("  Turns:       {turns}")));
            lines.push(Line::from(format!("  Summary:     {summary}")));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("\u{2500}".repeat(w), DIM)));
            lines.push(Line::from(""));
            lines
        }
        ServerMessage::Error { message } => {
            vec![Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("Error: {message}"), RED),
            ])]
        }
        // Non-visual messages
        ServerMessage::Spinner { .. }
        | ServerMessage::CommandList { .. }
        | ServerMessage::AgentMessageDelta { .. }
        | ServerMessage::TokenUsage { .. } => Vec::new(),
    }
}

/// Goodbye message for insert_before.
pub fn goodbye_lines() -> Vec<Line<'static>> {
    vec![Line::from(Span::styled("  Goodbye!", DIM))]
}

/// Render Lines into a Buffer (for insert_before).
pub fn render_lines_to_buffer(buf: &mut Buffer, lines: &[Line<'_>]) {
    let area = buf.area;
    for (i, line) in lines.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }
        buf.set_line(area.x, y, line, area.width);
    }
}
