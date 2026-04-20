//! Terminal rendering — stable multi-line redraw without cursor::position().

use std::io::{self, Write};
use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, SetAttribute, SetForegroundColor, ResetColor},
    terminal::{self, Clear, ClearType, BeginSynchronizedUpdate, EndSynchronizedUpdate},
};

use crate::app::{App, InputMode};
use crate::shimmer;
use decipher_protocol::ServerMessage;

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

pub fn draw_prompt(o: &mut io::Stdout, app: &mut App) -> io::Result<()> {
    // In pager mode, render the pager overlay instead
    if app.mode == InputMode::Pager {
        crate::pager::render_pager(o, app)?;
        return Ok(());
    }

    queue!(o, BeginSynchronizedUpdate)?;
    draw_prompt_inner(o, app)?;
    queue!(o, EndSynchronizedUpdate)?;
    o.flush()
}

/// Inner prompt rendering — no synchronized update wrapper.
/// Called directly by `commit_delta_lines` inside its own sync block.
pub fn draw_prompt_inner(o: &mut io::Stdout, app: &mut App) -> io::Result<()> {
    // Move cursor to top of prompt region (cursor is on the input line,
    // cursor_line_in_prompt tracks how far that is from the top).
    if app.cursor_line_in_prompt > 0 {
        queue!(o, cursor::MoveUp(app.cursor_line_in_prompt))?;
    }
    queue!(o, cursor::MoveToColumn(0))?;
    queue!(o, Clear(ClearType::FromCursorDown))?;

    let mut new_lines_above: u16 = 0;

    if app.mode == InputMode::CommandPopup {
        let filtered = app.filtered_commands();
        let max_show = filtered.len().min(10);
        let needed = max_show as u16 + if filtered.len() > 10 { 1 } else { 0 };
        if needed > app.owned_lines_above {
            let extra = needed - app.owned_lines_above;
            for _ in 0..extra { queue!(o, Print("\r\n"))?; }
            queue!(o, cursor::MoveUp(extra))?;
            queue!(o, cursor::MoveToColumn(0))?;
        }
        for (i, cmd) in filtered.iter().take(max_show).enumerate() {
            if i == app.popup_index {
                plain(o, "  ")?; bold_cyan(o, &format!("{:<14}", cmd.name))?; white(o, &cmd.description)?;
            } else {
                plain(o, "  ")?; cyan(o, &format!("{:<14}", cmd.name))?; dim(o, &cmd.description)?;
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

    // File search popup (@)
    if app.mode == InputMode::FileSearch && !app.file_search_results.is_empty() {
        let max_show = app.file_search_results.len().min(10);
        for (i, result) in app.file_search_results.iter().take(max_show).enumerate() {
            let icon = if result.is_dir { "📁" } else if result.is_image { "🖼" } else { "  " };
            if i == app.file_search_index {
                plain(o, "  ")?; bold_cyan(o, icon)?; plain(o, " ")?; bold_cyan(o, &result.path)?;
            } else {
                plain(o, "  ")?; dim(o, icon)?; plain(o, " ")?; dim(o, &result.path)?;
            }
            queue!(o, Print("\r\n"))?;
            new_lines_above += 1;
        }
        if app.file_search_results.len() > max_show {
            dim(o, &format!("  … {} more", app.file_search_results.len() - max_show))?;
            queue!(o, Print("\r\n"))?;
            new_lines_above += 1;
        }
    }

    // Streaming delta preview (partial line not yet committed)
    if app.stream.active && !app.stream.partial_line().is_empty() {
        plain(o, "  ")?;
        plain(o, app.stream.partial_line())?;
        queue!(o, Print("\r\n"))?;
        new_lines_above += 1;
    }

    // Prompt line(s) — multi-line support
    let input_lines: Vec<&str> = app.input.split('\n').collect();
    let input_line_count = input_lines.len();
    for (i, line) in input_lines.iter().enumerate() {
        if i == 0 { dim(o, "│ ")?; bold_yellow(o, "❯ ")?; }
        else { dim(o, "│   ")?; }
        plain(o, line)?;
        if i < input_line_count - 1 { queue!(o, Print("\r\n"))?; new_lines_above += 1; }
    }

    // Spinner with shimmer animation
    if let Some(ref label) = app.spinner_label {
        // Time-based frame: 80ms per frame for comfortable reading speed.
        // Decoupled from tick rate so spinner looks the same at any FPS.
        let frame_idx = app.spinner_started
            .map(|t| (t.elapsed().as_millis() / 80) as usize)
            .unwrap_or(app.spinner_frame);
        let frame = SPINNER_FRAMES[frame_idx % SPINNER_FRAMES.len()];
        let elapsed = app.spinner_started
            .map(|t| format!(" ({:.1}s)", t.elapsed().as_secs_f64()))
            .unwrap_or_default();

        queue!(o, Print("\r\n"))?;
        plain(o, "  ")?;
        cyan(o, frame)?;
        plain(o, " ")?;

        // Shimmer effect on the label text
        let shimmer_text = shimmer::shimmer_chars(label);
        for (ch, color) in &shimmer_text {
            queue!(o, SetForegroundColor(*color), Print(ch.to_string()))?;
        }
        queue!(o, ResetColor)?;
        dim(o, &elapsed)?;
        new_lines_above += 1;
    }

    // Queued message indicator
    if app.queued_message.is_some() {
        queue!(o, Print("\r\n"))?;
        dim(o, "  ")?; yellow(o, "⏳")?; dim(o, " Message queued — will send when agent finishes")?;
        new_lines_above += 1;
    }

    // Footer hints — TRUNCATE to terminal width to prevent wrapping.
    // Line wrapping breaks move_up calculation (it counts logical lines, not screen rows).
    let term_width = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
    let hints = match app.mode {
        InputMode::ApprovalPending => "  y approve  a always  n deny  Esc cancel".to_string(),
        InputMode::CommandPopup => "  ↑↓ navigate  Enter select  Esc cancel".to_string(),
        InputMode::HistorySearch => {
            let status = if app.search_match_index.is_some() { "match" } else if app.search_query.is_empty() { "" } else { "no match" };
            format!("  (reverse-i-search)`{}': {}  Ctrl+R older  Enter accept  Esc cancel", app.search_query, status)
        }
        InputMode::FileSearch => "  ↑↓ navigate  Tab/Enter select  Esc cancel".to_string(),
        InputMode::Pager => "  j/k scroll  q quit".to_string(),
        InputMode::Normal => {
            let token_info = if app.total_tokens > 0 {
                format!("  [{} tokens]", format_tokens(app.total_tokens))
            } else {
                String::new()
            };
            if app.agent_busy { format!("  Ctrl+C interrupt{token_info}") }
            else { format!("  Enter submit  / commands  Ctrl+R search  Ctrl+C quit{token_info}") }
        }
    };
    // Truncate hints to terminal width — wrapping would break cursor tracking
    let hints_truncated: String = hints.chars().take(term_width.saturating_sub(1)).collect();
    queue!(o, Print("\r\n"))?; dim(o, &hints_truncated)?; new_lines_above += 1;

    // Shortcut overlay
    if app.show_shortcuts {
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
        queue!(o, Print("\r\n"))?;
        dim(o, "  ┌ SHORTCUTS ─────────────────────")?;
        new_lines_above += 1;
        for (key, desc) in &shortcuts {
            queue!(o, Print("\r\n"))?;
            dim(o, "  │ ")?; cyan(o, &format!("{:<16}", key))?; dim(o, desc)?;
            new_lines_above += 1;
        }
        queue!(o, Print("\r\n"))?;
        dim(o, "  └────────────────────────────────")?;
        new_lines_above += 1;
    }

    // Cursor positioning: move from end of output back up to the input cursor.
    //
    // CRITICAL: We must count actual SCREEN ROWS, not logical lines.
    // A logical line that wraps at the terminal edge becomes multiple screen rows.
    // If we only count logical lines, move_up is too small → cursor ends up wrong
    // → next redraw doesn't clear properly → ghost lines appear.

    let (cursor_line, cursor_col) = cursor_position_in_multiline(&app.input, app.cursor);

    // Screen rows for input lines BELOW the cursor line
    let mut input_rows_below: u16 = 0;
    for i in (cursor_line + 1)..input_line_count {
        let line_visible_width = input_lines[i].chars().count() + 4; // "│   " prefix
        input_rows_below += screen_rows(line_visible_width, term_width);
    }

    // Screen rows for spinner
    let spinner_rows: u16 = if app.spinner_label.is_some() { 1 } else { 0 };
    // Screen rows for queued indicator
    let queued_rows: u16 = if app.queued_message.is_some() { 1 } else { 0 };
    // Screen rows for hints (already truncated, so always 1)
    let hints_rows: u16 = 1;
    // Screen rows for shortcuts
    let shortcut_rows: u16 = if app.show_shortcuts { 19 } else { 0 };

    // Total screen rows from cursor to bottom of all output
    let move_up = input_rows_below + spinner_rows + queued_rows + hints_rows + shortcut_rows;

    if move_up > 0 {
        queue!(o, cursor::MoveUp(move_up))?;
    }
    queue!(o, cursor::MoveToColumn(0))?;
    let col = (cursor_col + 4) as u16; // 4 = "│ ❯ " prefix width
    if col > 0 { queue!(o, cursor::MoveRight(col))?; }

    app.owned_lines_above = new_lines_above;
    // cursor_line_in_prompt: screen rows from cursor to the TOP of the prompt.
    // Used by clear_prompt to move up the correct amount.
    app.cursor_line_in_prompt = new_lines_above.saturating_sub(move_up);

    // NOTE: Do NOT flush here — let the caller (draw_prompt) flush after
    // EndSynchronizedUpdate to ensure atomic rendering.
    Ok(())
}

pub fn clear_prompt(o: &mut io::Stdout, app: &mut App) -> io::Result<()> {
    // Move from cursor position (on the input line) up to the top of the prompt region.
    if app.cursor_line_in_prompt > 0 { queue!(o, cursor::MoveUp(app.cursor_line_in_prompt))?; }
    queue!(o, cursor::MoveToColumn(0), Clear(ClearType::FromCursorDown))?;
    app.owned_lines_above = 0;
    app.cursor_line_in_prompt = 0;
    o.flush() // flush immediately — callers outside sync blocks need this
}

pub fn print_banner(o: &mut io::Stdout, version: &str, provider: &str, model: &str, directory: &str, base_url: Option<&str>, api_key_set: bool) -> io::Result<()> {
    let url_disp = base_url.map(|u| { let t = if u.len() > 40 { format!("{}...", &u[..37]) } else { u.to_string() }; format!("  ({t})") }).unwrap_or_default();
    plain(o, "\r\n")?;
    plain(o, "    ")?; bold_yellow(o, "/\\_/\\")?; plain(o, "\r\n")?;
    plain(o, "   ")?; bold_yellow(o, "( •ᴥ• )")?; plain(o, "  ")?; bold_cyan(o, "DeCIpher")?; plain(o, " ")?; dim(o, &format!("v{version}"))?; plain(o, "\r\n\r\n")?;
    dim(o, "  provider   ")?; cyan(o, provider)?; if !url_disp.is_empty() { dim(o, &url_disp)?; } plain(o, "\r\n")?;
    dim(o, "  model      ")?; cyan(o, model)?; plain(o, "\r\n")?;
    dim(o, "  directory  ")?; white(o, directory)?; plain(o, "\r\n")?;
    dim(o, "  approval   ")?; cyan(o, "on-request")?; dim(o, "  last: ")?; dim(o, "idle")?; plain(o, "\r\n")?;
    dim(o, "  api key    ")?;
    if api_key_set { green(o, "●")?; dim(o, " configured")?; } else { red(o, "○")?; red(o, " not set")?; }
    plain(o, "\r\n\r\n")?;
    dim(o, "  Type a mission, paste a path, or ")?; cyan(o, "/help")?; dim(o, " for commands.")?; plain(o, "\r\n")?;
    dim(o, "  ctrl+r")?; dim(o, " history  ")?; dim(o, "ctrl+t")?; dim(o, " transcript  ")?; dim(o, "ctrl+c")?; dim(o, " quit")?; plain(o, "\r\n\r\n")?;
    o.flush()
}

pub fn print_user_input(o: &mut io::Stdout, text: &str) -> io::Result<()> {
    dim(o, "╭─")?; plain(o, "\r\n")?;
    dim(o, "│ ")?; bold_yellow(o, "❯ ")?; plain(o, text)?; plain(o, "\r\n")?;
    dim(o, "╰─")?; plain(o, "\r\n")?;
    o.flush()
}

fn section_header(o: &mut io::Stdout, label: &str, tone: &str) -> io::Result<()> {
    let w = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
    let dashes = w.saturating_sub(label.len() + 4).max(8);
    plain(o, "\r\n")?; dim(o, "┌ ")?;
    match tone { "agent" => bold_cyan(o, label)?, "command" => yellow(o, label)?, _ => bold(o, label)? }
    plain(o, " ")?; dim(o, &"─".repeat(dashes))?; plain(o, "\r\n")?;
    o.flush()
}

pub fn print_server_message(o: &mut io::Stdout, msg: &ServerMessage, app: &mut App) -> io::Result<()> {
    match msg {
        ServerMessage::Banner { version, provider, model, directory, api_key_set } => {
            print_banner(o, version, provider, model, directory, None, *api_key_set)?;
        }
        ServerMessage::Mission { understood, target, steps, .. } => {
            section_header(o, "MISSION", "agent")?;
            plain(o, "  ")?; bold_yellow(o, "Understood: ")?; white(o, understood)?; plain(o, "\r\n")?;
            if let Some(t) = target { plain(o, "  ")?; dim(o, "Target: ")?; cyan(o, t)?; plain(o, "\r\n")?; }
            if !steps.is_empty() {
                plain(o, "\r\n  ")?; dim(o, "Plan:")?; plain(o, "\r\n")?;
                for (i, s) in steps.iter().enumerate() { plain(o, "    ")?; dim(o, &format!("{}. ", i+1))?; plain(o, s)?; plain(o, "\r\n")?; }
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
            dim(o, "  ┌─ ")?; bold_yellow(o, "APPROVAL")?; dim(o, &format!(" {bar}\r\n"))?;
            dim(o, "  │\r\n")?;
            if let Some(act) = action {
                dim(o, "  │ ")?; bold(o, "Action: ")?; cyan(o, &act.tool)?; plain(o, "\r\n")?;
                if let Some(reason) = &act.reasoning { dim(o, "  │ ")?; dim(o, reason)?; plain(o, "\r\n")?; }
                dim(o, "  │\r\n")?;
            }
            dim(o, "  │ DeCIpher requests these capabilities:\r\n")?;
            for c in capabilities { dim(o, "  │   ")?; yellow(o, "› ")?; plain(o, c)?; plain(o, "\r\n")?; }
            dim(o, "  │\r\n")?;
            dim(o, "  │  Scope: this session only. Nothing pushed or deployed.\r\n")?;
            dim(o, "  │\r\n")?;
            dim(o, "  │  ")?; bold(o, "y")?; dim(o, " approve  ")?; bold(o, "a")?; dim(o, " always  ")?; bold(o, "n")?; dim(o, " deny")?; plain(o, "\r\n")?;
            dim(o, &format!("  └{bar}─\r\n"))?;
            o.flush()?;
        }
        ServerMessage::ToolStart { tool, reasoning } => {
            let r = &reasoning[..reasoning.len().min(60)];
            let ts_frame = app.spinner_started
                .map(|t| (t.elapsed().as_millis() / 80) as usize)
                .unwrap_or(0);
            plain(o, "  ")?; cyan(o, SPINNER_FRAMES[ts_frame % SPINNER_FRAMES.len()])?;
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
            if app.suppress_next_agent_message {
                app.suppress_next_agent_message = false;
            } else {
                decipher_markdown::render_markdown(o, text, 2)?;
            }
        }
        ServerMessage::MissionComplete { outcome, summary, turns, elapsed_ms, .. } => {
            let s = *elapsed_ms as f64 / 1000.0;
            let w = terminal::size().map(|(w,_)| w as usize).unwrap_or(80).min(60);
            plain(o, "\r\n")?; dim(o, &"─".repeat(w))?; plain(o, "\r\n\r\n")?;
            bold(o, "  [RESULT]")?; plain(o, "\r\n")?;
            plain(o, "  Outcome:     ")?;
            if outcome == "PASS" { green(o, &format!("PASS ({s:.1}s)"))?; } else { red(o, &format!("FAIL ({s:.1}s)"))?; }
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
        ServerMessage::AgentMessageDelta { .. } => {}
        ServerMessage::TokenUsage { .. } => {}
    }
    Ok(())
}

/// Run a commit tick on the streaming pipeline.
/// Uses adaptive chunking to decide how many lines to drain.
/// Clears prompt, prints drained lines, redraws prompt.
/// Returns true if any lines were rendered.
pub fn commit_delta_lines(o: &mut io::Stdout, app: &mut App) -> io::Result<bool> {
    use crate::streaming::DrainPlan;

    let plan = app.stream.commit_tick();
    let lines = match plan {
        DrainPlan::None => return Ok(false),
        DrainPlan::Single => app.stream.drain(1),
        DrainPlan::Batch(n) => app.stream.drain(n),
    };

    if lines.is_empty() { return Ok(false); }

    queue!(o, BeginSynchronizedUpdate)?;
    clear_prompt(o, app)?;

    for line in &lines {
        plain(o, "  ")?;
        plain(o, line)?;
        queue!(o, Print("\r\n"))?;
    }
    o.flush()?;

    draw_prompt_inner(o, app)?;

    queue!(o, EndSynchronizedUpdate)?;
    o.flush()?;

    Ok(true)
}

/// Flush all remaining stream content (queue + partial line). Called at end of stream.
pub fn flush_delta_buffer(o: &mut io::Stdout, app: &mut App) -> io::Result<()> {
    let lines = app.stream.drain_all();

    if lines.is_empty() {
        app.suppress_next_agent_message = true;
        return Ok(());
    }

    clear_prompt(o, app)?;

    for line in &lines {
        plain(o, "  ")?;
        plain(o, line)?;
        queue!(o, Print("\r\n"))?;
    }
    o.flush()?;

    app.suppress_next_agent_message = true;
    Ok(())
}

/// Send a desktop notification.
/// Uses OSC 9 on supported terminals (iTerm2, WezTerm, Kitty),
/// falls back to BEL on others.
pub fn send_notification(o: &mut io::Stdout, message: &str) -> io::Result<()> {
    let clean: String = message.chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    // Try OSC 9 first, then fall back to BEL
    write!(o, "\x1b]9;{clean}\x07")?;
    o.flush()
}

/// Send a BEL (audible bell) notification.
pub fn send_bell(o: &mut io::Stdout) -> io::Result<()> {
    write!(o, "\x07")?;
    o.flush()
}

/// Set terminal window title via OSC 0.
pub fn set_terminal_title(o: &mut io::Stdout, title: &str) -> io::Result<()> {
    // Sanitize: remove control chars, cap length
    let clean: String = title.chars()
        .filter(|c| !c.is_control())
        .take(240)
        .collect();
    write!(o, "\x1b]0;{clean}\x07")?;
    o.flush()
}

pub fn print_goodbye(o: &mut io::Stdout) -> io::Result<()> {
    dim(o, "  Goodbye!")?; plain(o, "\r\n")?; o.flush()
}

/// How many screen rows a string of `visible_chars` width occupies
/// in a terminal of `term_width` columns. Minimum 1.
fn screen_rows(visible_chars: usize, term_width: usize) -> u16 {
    if visible_chars == 0 || term_width == 0 { return 1; }
    ((visible_chars + term_width - 1) / term_width).max(1) as u16
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1_000_000.0) }
    else if n >= 1_000 { format!("{:.1}K", n as f64 / 1_000.0) }
    else { n.to_string() }
}

fn cursor_position_in_multiline(text: &str, cursor: usize) -> (usize, usize) {
    let mut line = 0;
    let mut col = 0;
    for (i, ch) in text.char_indices() {
        if i >= cursor { break; }
        if ch == '\n' { line += 1; col = 0; } else { col += 1; }
    }
    (line, col)
}
