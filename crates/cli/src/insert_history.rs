//! Insert history lines above the fixed viewport using Codex-style
//! Reverse Index + DECSTBM scroll regions.
//!
//! The algorithm has two passes:
//!
//! 1. **Make room** — if there is space below the viewport, set a scroll
//!    region from the viewport top to the screen bottom and emit Reverse
//!    Index (`ESC M`) to push the viewport down, creating empty rows
//!    above it.
//!
//! 2. **Fill** — set a scroll region from screen top to the (new) viewport
//!    top. Position the cursor at the bottom of that region and emit
//!    `\r\n` + styled content for each history line. The scroll region
//!    pushes the oldest lines into the terminal's native scrollback buffer.
//!
//! This approach is immune to resize ghost artifacts because:
//! - Content above the viewport is managed by the terminal emulator's
//!   native scrollback — the terminal repositions it during resize and
//!   there is nothing for us to re-render.
//! - The viewport position is tracked dynamically; on resize we adjust
//!   it via cursor-delta rather than recomputation.

use std::fmt;
use std::io::{self, Write};

use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{
    Attribute, Color as CColor, Colors, Print, SetAttribute, SetBackgroundColor, SetColors,
    SetForegroundColor,
};
use crossterm::terminal::{Clear, ClearType};
use crossterm::Command;
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;

// ── DECSTBM commands ────────────────────────────────────────────────────────

/// `ESC [ <top> ; <bottom> r` — set scroll region (1-based).
struct SetScrollRegion(std::ops::Range<u16>);

impl Command for SetScrollRegion {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[{};{}r", self.0.start, self.0.end)
    }
    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
    }
}

/// `ESC [ r` — reset scroll region to full screen.
struct ResetScrollRegion;

impl Command for ResetScrollRegion {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[r")
    }
    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Insert `lines` above the viewport at row `vp_y`.
///
/// Returns the number of rows the viewport was pushed down (the caller
/// must add this to `vp_y` and recreate the ratatui `Terminal` at the new
/// position).
///
/// `screen_height` is the total terminal rows.  `width` is the terminal
/// columns (used for line truncation).
pub fn insert_history_lines(
    lines: &[Line<'static>],
    vp_y: u16,
    screen_height: u16,
    width: u16,
) -> io::Result<u16> {
    if lines.is_empty() {
        return Ok(0);
    }

    let line_count = lines.len() as u16;
    let mut stdout = io::stdout();
    let mut viewport_shift: u16 = 0;

    // ── Pass 1: make room below viewport ────────────────────────────────
    // If there is space below the viewport, use Reverse Index to push the
    // viewport down, creating empty rows above it for the new history.
    let space_below = screen_height.saturating_sub(vp_y + 6); // 6 = viewport height
    if space_below > 0 {
        let shift = line_count.min(space_below);

        // Set scroll region from viewport-top to screen-bottom (1-based).
        let top_1based = vp_y + 1;
        queue!(stdout, SetScrollRegion(top_1based..screen_height))?;
        queue!(stdout, MoveTo(0, vp_y))?;

        // Reverse Index (ESC M) at the top of the scroll region pushes
        // content DOWN within the region, freeing rows above.
        for _ in 0..shift {
            queue!(stdout, Print("\x1bM"))?;
        }

        queue!(stdout, ResetScrollRegion)?;
        viewport_shift = shift;
    }

    // The effective viewport top after shifting.
    let new_vp_y = vp_y + viewport_shift;

    // ── Pass 2: write history lines ─────────────────────────────────────
    // Set scroll region to the area ABOVE the viewport (1-based rows
    // 1..new_vp_y). Position cursor at the bottom of that region and
    // emit \r\n + content for each line.
    if new_vp_y > 0 {
        queue!(stdout, SetScrollRegion(1..new_vp_y))?;

        let cursor_top = new_vp_y.saturating_sub(1);
        queue!(stdout, MoveTo(0, cursor_top))?;

        for line in lines {
            queue!(stdout, Print("\r\n"))?;
            write_history_line(&mut stdout, line, width)?;
        }

        queue!(stdout, ResetScrollRegion)?;
    }

    // Restore cursor to the viewport area.
    queue!(stdout, MoveTo(0, new_vp_y))?;
    stdout.flush()?;

    Ok(viewport_shift)
}

// ── Line rendering ──────────────────────────────────────────────────────────

/// Render a single styled `Line` to raw stdout.
///
/// Clears the current line, sets per-span foreground/background colors and
/// text modifiers, and prints each span's content. Resets all attributes
/// at the end.
fn write_history_line(stdout: &mut io::Stdout, line: &Line<'static>, width: u16) -> io::Result<()> {
    // Set line-level colors (merged into spans below).
    queue!(
        stdout,
        SetColors(Colors::new(
            line.style.fg.map(ratatui_to_crossterm).unwrap_or(CColor::Reset),
            line.style.bg.map(ratatui_to_crossterm).unwrap_or(CColor::Reset),
        ))
    )?;
    queue!(stdout, Clear(ClearType::UntilNewLine))?;

    // Merge line-level style into each span so that line-level fg/bg
    // (e.g. blockquote green) is visible on every span.
    let mut fg = Color::Reset;
    let mut bg = Color::Reset;
    let mut last_modifier = Modifier::empty();

    let max_col = width as usize;
    let mut col: usize = 0;

    for span in &line.spans {
        if col >= max_col {
            break;
        }

        let merged = span.style.patch(line.style);
        let next_fg = merged.fg.unwrap_or(Color::Reset);
        let next_bg = merged.bg.unwrap_or(Color::Reset);

        // Colors
        if next_fg != fg || next_bg != bg {
            queue!(
                stdout,
                SetColors(Colors::new(
                    ratatui_to_crossterm(next_fg),
                    ratatui_to_crossterm(next_bg),
                ))
            )?;
            fg = next_fg;
            bg = next_bg;
        }

        // Modifiers — diff-based (add what's new, remove what's gone)
        let mut modifier = Modifier::empty();
        modifier.insert(merged.add_modifier);
        modifier.remove(merged.sub_modifier);
        if modifier != last_modifier {
            queue_modifier_diff(stdout, last_modifier, modifier)?;
            last_modifier = modifier;
        }

        let content = span.content.as_ref();
        let char_count = content.chars().count();
        let remaining = max_col.saturating_sub(col);
        if char_count <= remaining {
            queue!(stdout, Print(content))?;
            col += char_count;
        } else {
            // Truncate to fit remaining columns
            let truncated: String = content.chars().take(remaining).collect();
            col += remaining;
            queue!(stdout, Print(truncated))?;
        }
    }

    // Reset all attributes.
    queue!(
        stdout,
        SetForegroundColor(CColor::Reset),
        SetBackgroundColor(CColor::Reset),
        SetAttribute(Attribute::Reset),
    )
}

/// Emit only the ANSI attribute changes needed to go from `from` to `to`.
fn queue_modifier_diff(stdout: &mut io::Stdout, from: Modifier, to: Modifier) -> io::Result<()> {
    let removed = from - to;
    if removed.contains(Modifier::BOLD) || removed.contains(Modifier::DIM) {
        queue!(stdout, SetAttribute(Attribute::NormalIntensity))?;
        // NormalIntensity clears both Bold and Dim; re-add if still needed.
        if to.contains(Modifier::DIM) {
            queue!(stdout, SetAttribute(Attribute::Dim))?;
        }
        if to.contains(Modifier::BOLD) {
            queue!(stdout, SetAttribute(Attribute::Bold))?;
        }
    }
    if removed.contains(Modifier::ITALIC) {
        queue!(stdout, SetAttribute(Attribute::NoItalic))?;
    }
    if removed.contains(Modifier::UNDERLINED) {
        queue!(stdout, SetAttribute(Attribute::NoUnderline))?;
    }

    let added = to - from;
    if added.contains(Modifier::BOLD) {
        queue!(stdout, SetAttribute(Attribute::Bold))?;
    }
    if added.contains(Modifier::DIM) {
        queue!(stdout, SetAttribute(Attribute::Dim))?;
    }
    if added.contains(Modifier::ITALIC) {
        queue!(stdout, SetAttribute(Attribute::Italic))?;
    }
    if added.contains(Modifier::UNDERLINED) {
        queue!(stdout, SetAttribute(Attribute::Underlined))?;
    }
    Ok(())
}

/// Convert a ratatui `Color` to a crossterm `Color`.
fn ratatui_to_crossterm(c: Color) -> CColor {
    match c {
        Color::Reset => CColor::Reset,
        Color::Black => CColor::Black,
        Color::Red => CColor::DarkRed,
        Color::Green => CColor::DarkGreen,
        Color::Yellow => CColor::DarkYellow,
        Color::Blue => CColor::DarkBlue,
        Color::Magenta => CColor::DarkMagenta,
        Color::Cyan => CColor::DarkCyan,
        Color::Gray => CColor::Grey,
        Color::DarkGray => CColor::DarkGrey,
        Color::LightRed => CColor::Red,
        Color::LightGreen => CColor::Green,
        Color::LightYellow => CColor::Yellow,
        Color::LightBlue => CColor::Blue,
        Color::LightMagenta => CColor::Magenta,
        Color::LightCyan => CColor::Cyan,
        Color::White => CColor::White,
        Color::Rgb(r, g, b) => CColor::Rgb { r, g, b },
        Color::Indexed(i) => CColor::AnsiValue(i),
    }
}
