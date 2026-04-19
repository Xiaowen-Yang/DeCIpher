//! Lightweight markdown-to-ANSI renderer.
//!
//! Handles the subset of markdown that appears in agent output:
//! - **bold**, *italic*, `inline code`
//! - ```code blocks``` (with optional language tag)
//! - Bullet lists (-, *, +)
//! - Headers (#, ##, ###)
//!
//! Returns styled text using ANSI escape sequences.

use std::io::{self, Write};
use crossterm::{
    queue,
    style::{Attribute, Color, Print, SetAttribute, SetForegroundColor, ResetColor},
};

/// Shiba Inu fur color — warm orange-yellow (#E8A317).
const SHIBA: Color = Color::Rgb { r: 232, g: 163, b: 23 };

/// Render a markdown string to the terminal with ANSI styling.
/// Each line is indented by `indent` spaces.
pub fn render_markdown(o: &mut io::Stdout, text: &str, indent: usize) -> io::Result<()> {
    let prefix = " ".repeat(indent);
    let mut in_code_block = false;

    for line in text.lines() {
        queue!(o, Print(&prefix))?;

        if line.starts_with("```") {
            in_code_block = !in_code_block;
            if in_code_block {
                // Opening fence — show language tag dimmed
                let lang = line.trim_start_matches('`').trim();
                if !lang.is_empty() {
                    queue!(o, SetAttribute(Attribute::Dim), Print(lang), SetAttribute(Attribute::Reset))?;
                }
            }
            queue!(o, Print("\r\n"))?;
            continue;
        }

        if in_code_block {
            // Code block content: green, no inline styling
            queue!(o,
                SetForegroundColor(Color::Green),
                Print(line),
                ResetColor,
                Print("\r\n"),
            )?;
            continue;
        }

        // Headers
        if let Some(rest) = line.strip_prefix("### ") {
            queue!(o, SetAttribute(Attribute::Bold), SetForegroundColor(Color::Cyan),
                Print(rest), ResetColor, SetAttribute(Attribute::Reset), Print("\r\n"))?;
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            queue!(o, SetAttribute(Attribute::Bold), SetForegroundColor(Color::Cyan),
                Print(rest), ResetColor, SetAttribute(Attribute::Reset), Print("\r\n"))?;
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            queue!(o, SetAttribute(Attribute::Bold), SetForegroundColor(Color::Cyan),
                Print(rest), ResetColor, SetAttribute(Attribute::Reset), Print("\r\n"))?;
            continue;
        }

        // Bullet lists
        let bullet_rest = line.strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .or_else(|| line.strip_prefix("+ "));
        if let Some(rest) = bullet_rest {
            queue!(o, SetForegroundColor(SHIBA), Print("• "), ResetColor)?;
            render_inline(o, rest)?;
            queue!(o, Print("\r\n"))?;
            continue;
        }

        // Numbered lists (e.g. "1. item")
        if line.len() > 2 {
            let dot_pos = line.find(". ");
            if let Some(pos) = dot_pos {
                if pos <= 3 && line[..pos].chars().all(|c| c.is_ascii_digit()) {
                    let number = &line[..pos + 2];
                    let rest = &line[pos + 2..];
                    queue!(o, SetAttribute(Attribute::Dim), Print(number), SetAttribute(Attribute::Reset))?;
                    render_inline(o, rest)?;
                    queue!(o, Print("\r\n"))?;
                    continue;
                }
            }
        }

        // Regular line with inline styling
        render_inline(o, line)?;
        queue!(o, Print("\r\n"))?;
    }

    o.flush()
}

/// Render inline markdown elements: **bold**, *italic*, `code`.
fn render_inline(o: &mut impl Write, text: &str) -> io::Result<()> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // **bold**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_closing(&chars, i + 2, "**") {
                let inner: String = chars[i + 2..end].iter().collect();
                queue!(o, SetAttribute(Attribute::Bold), Print(&inner), SetAttribute(Attribute::Reset))?;
                i = end + 2;
                continue;
            }
        }

        // *italic*
        if chars[i] == '*' && (i + 1 < len && chars[i + 1] != '*') {
            if let Some(end) = find_closing_char(&chars, i + 1, '*') {
                let inner: String = chars[i + 1..end].iter().collect();
                queue!(o, SetAttribute(Attribute::Italic), Print(&inner), SetAttribute(Attribute::Reset))?;
                i = end + 1;
                continue;
            }
        }

        // `inline code`
        if chars[i] == '`' {
            if let Some(end) = find_closing_char(&chars, i + 1, '`') {
                let inner: String = chars[i + 1..end].iter().collect();
                queue!(o, SetForegroundColor(Color::Green), Print(&inner), ResetColor)?;
                i = end + 1;
                continue;
            }
        }

        // Regular character
        queue!(o, Print(chars[i].to_string()))?;
        i += 1;
    }

    Ok(())
}

/// Find closing delimiter (two-char, e.g. "**") starting from `start`.
fn find_closing(chars: &[char], start: usize, delim: &str) -> Option<usize> {
    let dc: Vec<char> = delim.chars().collect();
    for i in start..chars.len().saturating_sub(dc.len() - 1) {
        if chars[i..i + dc.len()] == dc[..] {
            return Some(i);
        }
    }
    None
}

/// Find closing single-char delimiter starting from `start`.
fn find_closing_char(chars: &[char], start: usize, delim: char) -> Option<usize> {
    for i in start..chars.len() {
        if chars[i] == delim {
            return Some(i);
        }
    }
    None
}
