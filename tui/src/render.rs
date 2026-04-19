//! Terminal rendering — stable multi-line redraw without cursor::position().
//!
//! Technique: we track `app.owned_lines_above` — how many lines above the
//! prompt we printed (popup). To redraw:
//!   1. MoveUp(owned_lines_above) — go to top of our region
//!   2. MoveToColumn(0)
//!   3. Clear(FromCursorDown) — erase everything below
//!   4. Print new content (popup lines + prompt line)
//!   5. Update owned_lines_above
//!
//! This never calls cursor::position(). All state is tracked in `app`.

use std::io::{self, Write};
use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, SetAttribute, SetForegroundColor, ResetColor},
    terminal::{self, Clear, ClearType, BeginSynchronizedUpdate, EndSynchronizedUpdate},
};

use crate::app::{App, InputMode};
use crate::protocol::ServerMessage;

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// ── Color helpers ────────────────────────────────────────────────────────────

/// Shiba Inu fur color — warm orange-yellow (#E8A317).
const SHIBA: Color = Color::Rgb { r: 232, g: 163, b: 23 };

fn dim(o: &mut impl Write, t: &str) -> io::Result<()> {
    queue!(o, SetAttribute(Attribute::Dim), Print(t), SetAttribute(Attribute::Reset))
}
fn cyan(o: &mut impl Write, t: &str) -> io::Result<()> {
    queue!(o, SetForegroundColor(Color::Cyan), Print(t), ResetColor)
}
fn bold_cyan(o: &mut impl Write, t: &str) -> io::Result<()> {
    queue!(o, SetAttribute(Attribute::Bold), SetForegroundColor(Color::Cyan), Print(t), ResetColor, SetAttribute(Attribute::Reset))
}
fn bold_yellow(o: &mut impl Write, t: &str) -> io::Result<()> {
    queue!(o, SetAttribute(Attribute::Bold), SetForegroundColor(SHIBA), Print(t), ResetColor, SetAttribute(Attribute::Reset))
}
fn yellow(o: &mut impl Write, t: &str) -> io::Result<()> {
    queue!(o, SetForegroundColor(SHIBA), Print(t), ResetColor)
}
fn green(o: &mut impl Write, t: &str) -> io::Result<()> {
    queue!(o, SetForegroundColor(Color::Green), Print(t), ResetColor)
}
fn red(o: &mut impl Write, t: &str) -> io::Result<()> {
    queue!(o, SetForegroundColor(Color::Red), Print(t), ResetColor)
}
fn white(o: &mut impl Write, t: &str) -> io::Result<()> {
    queue!(o, SetForegroundColor(Color::White), Print(t), ResetColor)
}
fn bold(o: &mut impl Write, t: &str) -> io::Result<()> {
    queue!(o, SetAttribute(Attribute::Bold), Print(t), SetAttribute(Attribute::Reset))
}
fn plain(o: &mut impl Write, t: &str) -> io::Result<()> {
    queue!(o, Print(t))
}

// ── Core redraw ──────────────────────────────────────────────────────────────
//
// Called on every keystroke. Erases our owned region and redraws.
// The cursor always ends on the prompt line after this call.

pub fn draw_prompt(o: &mut io::Stdout, app: &mut App) -> io::Result<()> {
    queue!(o, BeginSynchronizedUpdate)?;

    // Step 1: Move to top of our owned region
    if app.owned_lines_above > 0 {
        queue!(o, cursor::MoveUp(app.owned_lines_above))?;
    }
    queue!(o, cursor::MoveToColumn(0))?;

    // Step 2: Clear from here to end of screen
    queue!(o, Clear(ClearType::FromCursorDown))?;

    // Step 3: Calculate new popup content
    let mut new_lines_above: u16 = 0;

    if app.mode == InputMode::CommandPopup {
        let filtered = app.filtered_commands();
        let max_show = filtered.len().min(10);

        // If popup needs more space than we had before, scroll down first
        let needed = max_show as u16 + if filtered.len() > 10 { 1 } else { 0 };
        if needed > app.owned_lines_above {
            let extra = needed - app.owned_lines_above;
            for _ in 0..extra {
                queue!(o, Print("\r\n"))?;
            }
            queue!(o, cursor::MoveUp(extra))?;
            queue!(o, cursor::MoveToColumn(0))?;
        }

        for (i, cmd) in filtered.iter().take(max_show).enumerate() {
            if i == app.popup_index {
                plain(o, "  ")?;
                bold_cyan(o, &format!("{:<14}", cmd.name))?;
                white(o, &cmd.description)?;
            } else {
                plain(o, "  ")?;
                cyan(o, &format!("{:<14}", cmd.name))?;
                dim(o, &cmd.description)?;
            }
            queue!(o, Print("\r\n"))?;
            new_lines_above += 1;
        }
        if filtered.len() > max_show {
            dim(o, &format!("  … {} more", filtered.len() - max_show))?;
            queue!(o, Print("\r\n"))?;
            new_lines_above += 1;
        }
    }

    // Step 4: Draw prompt line(s) — supports multi-line input
    let input_lines: Vec<&str> = app.input.split('\n').collect();
    let input_line_count = input_lines.len();

    for (i, line) in input_lines.iter().enumerate() {
        if i == 0 {
            dim(o, "│ ")?;
            bold_yellow(o, "❯ ")?;
        } else {
            dim(o, "│   ")?; // continuation indent matching "│ ❯ "
        }
        plain(o, line)?;
        if i < input_line_count - 1 {
            queue!(o, Print("\r\n"))?;
            new_lines_above += 1;
        }
    }

    // Step 5: Draw spinner (if agent is busy)
    if let Some(ref label) = app.spinner_label {
        let frame = SPINNER_FRAMES[app.spinner_frame % SPINNER_FRAMES.len()];
        let elapsed = app.spinner_started.map(|t| {
            let s = t.elapsed().as_secs_f64();
            format!(" ({s:.1}s)")
        }).unwrap_or_default();
        queue!(o, Print("\r\n"))?;
        plain(o, "  ")?;
        cyan(o, frame)?;
        plain(o, " ")?;
        dim(o, label)?;
        dim(o, &elapsed)?;
        new_lines_above += 1;
    }

    // Step 6: Draw footer hints
    let hints = match app.mode {
        InputMode::ApprovalPending => "  y approve  n deny",
        InputMode::CommandPopup => "  ↑↓ navigate  Enter select  Esc cancel",
        InputMode::Normal => {
            if app.agent_busy {
                "  Ctrl+C interrupt"
            } else if input_line_count > 1 {
                "  Enter submit  Shift+Enter newline  Ctrl+C quit"
                } else {
                "  Enter submit  / commands  Ctrl+C quit"
            }
        }
    };
    queue!(o, Print("\r\n"))?;
    dim(o, hints)?;
    new_lines_above += 1;

    // Step 7: Position cursor on the correct input line
    // Cursor is N lines above bottom (hints line + remaining input lines + spinner)
    let spinner_lines: u16 = if app.spinner_label.is_some() { 1 } else { 0 };
    // Find which input line the cursor is on and the column within that line
    let (cursor_line, cursor_col) = cursor_position_in_multiline(&app.input, app.cursor);
    let lines_below_cursor = (input_line_count - 1 - cursor_line) as u16;
    let move_up = 1 + lines_below_cursor + spinner_lines; // 1 for hints line
    queue!(o, cursor::MoveUp(move_up))?;
    queue!(o, Print("\r"))?;
    let prefix_width = if cursor_line == 0 { 4 } else { 4 }; // "│ ❯ " or "│   "
    let col = (cursor_col + prefix_width) as u16;
    if col > 0 {
        queue!(o, cursor::MoveRight(col))?;
    }

    // Step 7: Update state
    app.owned_lines_above = new_lines_above;

    queue!(o, EndSynchronizedUpdate)?;
    o.flush()
}

// ── Clear our region (before printing agent output or on submit) ─────────────

pub fn clear_prompt(o: &mut io::Stdout, app: &mut App) -> io::Result<()> {
    if app.owned_lines_above > 0 {
        queue!(o, cursor::MoveUp(app.owned_lines_above))?;
    }
    queue!(o, cursor::MoveToColumn(0), Clear(ClearType::FromCursorDown))?;
    app.owned_lines_above = 0;
    o.flush()
}

// ── Banner ───────────────────────────────────────────────────────────────────

pub fn print_banner(
    o: &mut io::Stdout,
    version: &str, provider: &str, model: &str, directory: &str,
    base_url: Option<&str>, api_key_set: bool,
) -> io::Result<()> {
    let url_disp = base_url
        .map(|u| {
            let t = if u.len() > 40 { format!("{}...", &u[..37]) } else { u.to_string() };
            format!("  ({t})")
        })
        .unwrap_or_default();

    plain(o, "\r\n")?;
    plain(o, "    ")?; bold_yellow(o, "/\\_/\\")?; plain(o, "\r\n")?;
    plain(o, "   ")?; bold_yellow(o, "( •ᴥ• )")?; plain(o, "  ")?;
    bold_cyan(o, "DeCIpher")?; plain(o, " ")?;
    dim(o, &format!("v{version}"))?; plain(o, "\r\n\r\n")?;
    dim(o, "  provider   ")?; cyan(o, provider)?;
    if !url_disp.is_empty() { dim(o, &url_disp)?; }
    plain(o, "\r\n")?;
    dim(o, "  model      ")?; cyan(o, model)?; plain(o, "\r\n")?;
    dim(o, "  directory  ")?; white(o, directory)?; plain(o, "\r\n")?;
    dim(o, "  approval   ")?; cyan(o, "on-request")?;
    dim(o, "  last: ")?; dim(o, "idle")?; plain(o, "\r\n")?;
    dim(o, "  api key    ")?;
    if api_key_set { green(o, "●")?; dim(o, " configured")?; }
    else { red(o, "○")?; red(o, " not set")?; }
    plain(o, "\r\n\r\n")?;
    dim(o, "  Type a mission, paste a path, or ")?;
    cyan(o, "/help")?; dim(o, " for commands.")?; plain(o, "\r\n")?;
    dim(o, "  ctrl+r")?; dim(o, " history  ")?;
    dim(o, "ctrl+c")?; dim(o, " quit")?; plain(o, "\r\n\r\n")?;
    o.flush()
}

// ── User input (after submit) ────────────────────────────────────────────────

pub fn print_user_input(o: &mut io::Stdout, text: &str) -> io::Result<()> {
    dim(o, "╭─")?; plain(o, "\r\n")?;
    dim(o, "│ ")?; bold_yellow(o, "❯ ")?; plain(o, text)?; plain(o, "\r\n")?;
    dim(o, "╰─")?; plain(o, "\r\n")?;
    o.flush()
}

// ── Section header ───────────────────────────────────────────────────────────

fn section_header(o: &mut io::Stdout, label: &str, tone: &str) -> io::Result<()> {
    let w = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
    let dashes = w.saturating_sub(label.len() + 4).max(8);
    plain(o, "\r\n")?;
    dim(o, "┌ ")?;
    match tone { "agent" => bold_cyan(o, label)?, "command" => yellow(o, label)?, _ => bold(o, label)? }
    plain(o, " ")?; dim(o, &"─".repeat(dashes))?; plain(o, "\r\n")?;
    o.flush()
}

// ── Server messages ──────────────────────────────────────────────────────────

pub fn print_server_message(o: &mut io::Stdout, msg: &ServerMessage, app: &mut App) -> io::Result<()> {
    match msg {
        ServerMessage::Banner { version, provider, model, directory, api_key_set } => {
            print_banner(o, version, provider, model, directory, None, *api_key_set)?;
        }
        ServerMessage::Mission { understood, target, steps, .. } => {
            section_header(o, "MISSION", "agent")?;
            plain(o, "  ")?; bold_yellow(o, "Understood: ")?; white(o, understood)?; plain(o, "\r\n")?;
            if let Some(t) = target {
                plain(o, "  ")?; dim(o, "Target: ")?; cyan(o, t)?; plain(o, "\r\n")?;
            }
            if !steps.is_empty() {
                plain(o, "\r\n  ")?; dim(o, "Plan:")?; plain(o, "\r\n")?;
                for (i, s) in steps.iter().enumerate() {
                    plain(o, "    ")?; dim(o, &format!("{}. ", i+1))?; plain(o, s)?; plain(o, "\r\n")?;
                }
            }
            plain(o, "\r\n")?; o.flush()?;
        }
        ServerMessage::Clarification { question } => {
            section_header(o, "CLARIFICATION NEEDED", "command")?;
            plain(o, "  ")?; bold_yellow(o, "DeCIpher asks: ")?; white(o, question)?; plain(o, "\r\n")?;
            dim(o, "  Reply below and DeCIpher will continue.\r\n\r\n")?; o.flush()?;
        }
        ServerMessage::ApprovalRequest { action, capabilities } => {
            plain(o, "\r\n")?;
            let w = terminal::size().map(|(w, _)| w as usize).unwrap_or(80).min(60);
            let bar = "─".repeat(w.saturating_sub(4));
            dim(o, &format!("  ┌─ "))?; bold_yellow(o, "APPROVAL")?; dim(o, &format!(" {bar}\r\n"))?;
            dim(o, "  │\r\n")?;

            // Show specific action if available
            if let Some(act) = action {
                dim(o, "  │ ")?; bold(o, "Action: ")?; cyan(o, &act.tool)?; plain(o, "\r\n")?;
                if let Some(reason) = &act.reasoning {
                    dim(o, "  │ ")?; dim(o, reason)?; plain(o, "\r\n")?;
                }
                dim(o, "  │\r\n")?;
            }

            dim(o, "  │ DeCIpher requests these capabilities:\r\n")?;
            for c in capabilities {
                dim(o, "  │   ")?; yellow(o, "› ")?; plain(o, c)?; plain(o, "\r\n")?;
            }
            dim(o, "  │\r\n")?;
            dim(o, "  │  Scope: this session only. Nothing pushed or deployed.\r\n")?;
            dim(o, "  │\r\n")?;
            dim(o, "  │  ")?; bold(o, "y")?; dim(o, " approve  ")?; bold(o, "n")?; dim(o, " deny")?; plain(o, "\r\n")?;
            dim(o, &format!("  └{bar}─\r\n"))?;
            o.flush()?;
        }
        ServerMessage::ToolStart { tool, reasoning } => {
            let r = &reasoning[..reasoning.len().min(60)];
            plain(o, "  ")?; cyan(o, SPINNER_FRAMES[app.spinner_frame % SPINNER_FRAMES.len()])?;
            plain(o, " ")?; dim(o, &format!("{tool} — {r}"))?; plain(o, "\r\n")?; o.flush()?;
        }
        ServerMessage::ToolResult { tool, success, summary, elapsed_ms } => {
            let s = *elapsed_ms as f64 / 1000.0;
            plain(o, "  ")?;
            if *success { green(o, "✓")?; } else { red(o, "✗")?; }
            plain(o, &format!(" {tool} — {summary} "))?;
            dim(o, &format!("({s:.1}s)"))?; plain(o, "\r\n")?; o.flush()?;
        }
        ServerMessage::AgentMessage { text } => {
            crate::markdown::render_markdown(o, text, 2)?;
        }
        ServerMessage::MissionComplete { outcome, summary, turns, elapsed_ms, .. } => {
            let s = *elapsed_ms as f64 / 1000.0;
            let w = terminal::size().map(|(w,_)| w as usize).unwrap_or(80).min(60);
            plain(o, "\r\n")?; dim(o, &"─".repeat(w))?; plain(o, "\r\n\r\n")?;
            bold(o, "  [RESULT]")?; plain(o, "\r\n")?;
            plain(o, "  Outcome:     ")?;
            if outcome == "PASS" { green(o, &format!("PASS ({s:.1}s)"))?; }
            else { red(o, &format!("FAIL ({s:.1}s)"))?; }
            plain(o, "\r\n")?;
            plain(o, &format!("  Turns:       {turns}\r\n"))?;
            plain(o, &format!("  Summary:     {summary}\r\n\r\n"))?;
            dim(o, &"─".repeat(w))?; plain(o, "\r\n\r\n")?; o.flush()?;
        }
        ServerMessage::Error { message } => {
            plain(o, "  ")?; red(o, &format!("Error: {message}"))?; plain(o, "\r\n")?; o.flush()?;
        }
        ServerMessage::Spinner { .. } => {}
        ServerMessage::CommandList { .. } => {}
        ServerMessage::AgentMessageDelta { .. } => {
            // Handled directly in the main event loop for streaming
        }
    }
    Ok(())
}

pub fn print_goodbye(o: &mut io::Stdout) -> io::Result<()> {
    dim(o, "  Goodbye!")?; plain(o, "\r\n")?; o.flush()
}

/// Print a streaming text delta inline (no section header, no newline at end).
pub fn print_delta(o: &mut io::Stdout, text: &str) -> io::Result<()> {
    // Print each character, converting \n to \r\n for raw mode
    for ch in text.chars() {
        if ch == '\n' {
            queue!(o, Print("\r\n  "))?; // indent continuation
        } else {
            queue!(o, Print(ch.to_string()))?;
        }
    }
    o.flush()
}

/// Find (line_index, col_within_line) for a byte cursor in multi-line text.
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
