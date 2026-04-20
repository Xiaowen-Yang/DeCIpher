//! Pager overlay — full-screen transcript view (Ctrl+T).
//!
//! Displays committed chat history with Vim-style navigation.
//! j/k scroll, PgUp/PgDn, g/G for top/bottom, q/Esc to close.
//!
//! Codex ref: `codex-rs/tui/src/pager_overlay.rs`

use std::io::{self, Write};
use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, SetAttribute, SetForegroundColor, ResetColor},
    terminal::{self, Clear, ClearType},
};

use crate::app::{App, ChatEntryKind};

/// Render the pager overlay. Takes over the full terminal.
pub fn render_pager(o: &mut io::Stdout, app: &mut App) -> io::Result<()> {
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let height = height as usize;
    let width = width as usize;

    // Build all transcript lines
    let mut lines: Vec<TranscriptLine> = Vec::new();

    for entry in &app.history {
        let (prefix, color) = match entry.kind {
            ChatEntryKind::UserInput => ("❯ ", Color::Rgb { r: 232, g: 163, b: 23 }),
            ChatEntryKind::Mission => ("MISSION: ", Color::Cyan),
            ChatEntryKind::Clarification => ("? ", Color::Yellow),
            ChatEntryKind::ToolStart => ("⠋ ", Color::Cyan),
            ChatEntryKind::ToolResult => ("  ", Color::Green),
            ChatEntryKind::AgentMessage => ("  ", Color::Reset),
            ChatEntryKind::MissionComplete => ("✓ ", Color::Green),
            ChatEntryKind::Error => ("! ", Color::Red),
        };

        for text_line in entry.text.lines() {
            lines.push(TranscriptLine {
                prefix: prefix.to_string(),
                text: text_line.to_string(),
                color,
            });
        }
        lines.push(TranscriptLine {
            prefix: String::new(),
            text: String::new(),
            color: Color::Reset,
        });
    }

    // Clamp scroll
    let max_scroll = lines.len().saturating_sub(height.saturating_sub(2));
    if app.pager_scroll > max_scroll {
        app.pager_scroll = max_scroll;
    }

    // Clear screen (use inline clear, not alternate screen)
    queue!(o, cursor::MoveTo(0, 0), Clear(ClearType::All))?;

    // Header
    let header = format!(" TRANSCRIPT ({}/{} lines) — q to close ", app.pager_scroll + 1, lines.len());
    let header_pad = width.saturating_sub(header.len());
    queue!(o, SetAttribute(Attribute::Reverse))?;
    queue!(o, Print(&header), Print(" ".repeat(header_pad)))?;
    queue!(o, SetAttribute(Attribute::Reset), Print("\r\n"))?;

    // Content lines
    let visible_lines = height.saturating_sub(2);
    let start = app.pager_scroll;
    let end = (start + visible_lines).min(lines.len());

    for line in &lines[start..end] {
        queue!(o, SetForegroundColor(line.color))?;
        let display = if line.prefix.is_empty() && line.text.is_empty() {
            String::new()
        } else {
            let mut s = format!("{}{}", line.prefix, line.text);
            if s.len() > width {
                s.truncate(width.saturating_sub(1));
                s.push('…');
            }
            s
        };
        queue!(o, Print(&display))?;
        queue!(o, ResetColor, Print("\r\n"))?;
    }

    // Fill remaining lines
    for _ in (end - start)..visible_lines {
        queue!(o, Print("~\r\n"))?;
    }

    // Footer
    let footer = " j/k scroll  d/u half-page  g/G top/bottom  PgUp/PgDn  q quit ";
    let footer_pad = width.saturating_sub(footer.len());
    queue!(o, SetAttribute(Attribute::Reverse))?;
    queue!(o, Print(footer), Print(" ".repeat(footer_pad)))?;
    queue!(o, SetAttribute(Attribute::Reset))?;

    o.flush()
}

struct TranscriptLine {
    prefix: String,
    text: String,
    color: Color,
}
