//! Pager overlay — full-screen transcript view (Ctrl+T).
//!
//! Displays committed chat history (from ChatWidget cells) with Vim-style
//! navigation. j/k scroll, PgUp/PgDn, g/G for top/bottom, q/Esc to close.
//!
//! Phase 3: reads from ChatWidget::transcript_lines() instead of App::history.
//! Codex ref: `codex-rs/tui/src/pager_overlay.rs`

use std::io::{self, Write};
use crossterm::{
    cursor, queue,
    style::{Attribute, Print, SetAttribute},
    terminal::{self, Clear, ClearType},
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::app::App;

/// Render the pager overlay. Takes over the full terminal.
pub fn render_pager(o: &mut io::Stdout, app: &mut App) -> io::Result<()> {
    // CURSOR-2: use cached width from app state as fallback so the pager
    // never issues a blocking size query that can time out during active resize.
    let (width, height) = terminal::size().unwrap_or((app.chat.width(), 24));
    let height = height as usize;
    let width = width as usize;

    // Build transcript lines only when the cache is stale.
    // Cache key = (committed_cells.len(), active_cell_revision + animation_tick).
    let current_key = app.chat.transcript_cache_key();
    let current_width = width as u16;

    if app.pager_cache_key != current_key || app.pager_cache_width != current_width {
        let ratatui_lines = app.chat.transcript_lines(current_width);
        app.pager_cache = ratatui_lines.iter()
            .map(|line| {
                let mut buf = Buffer::empty(Rect::new(0, 0, current_width, 1));
                buf.set_line(0, 0, line, current_width);
                let mut s = String::with_capacity(width);
                for x in 0..current_width {
                    let cell = &buf[(x, 0)];
                    s.push_str(cell.symbol());
                }
                s.trim_end().to_string()
            })
            .collect();
        app.pager_cache_key = current_key;
        app.pager_cache_width = current_width;
    }

    let lines = &app.pager_cache;

    // Clamp scroll
    let max_scroll = lines.len().saturating_sub(height.saturating_sub(2));
    if app.pager_scroll > max_scroll {
        app.pager_scroll = max_scroll;
    }

    // Clear screen
    queue!(o, cursor::MoveTo(0, 0), Clear(ClearType::All))?;

    // Header
    let header = format!(" TRANSCRIPT ({}/{} lines) \u{2014} q to close ", app.pager_scroll + 1, lines.len());
    let header_pad = width.saturating_sub(header.len());
    queue!(o, SetAttribute(Attribute::Reverse))?;
    queue!(o, Print(&header), Print(" ".repeat(header_pad)))?;
    queue!(o, SetAttribute(Attribute::Reset), Print("\r\n"))?;

    // Content lines
    let visible_lines = height.saturating_sub(2);
    let start = app.pager_scroll;
    let end = (start + visible_lines).min(lines.len());

    for line in &lines[start..end] {
        let display = if line.len() > width {
            let mut s = line[..width.saturating_sub(1)].to_string();
            s.push('\u{2026}');
            s
        } else {
            line.clone()
        };
        queue!(o, Print(&display), Print("\r\n"))?;
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
