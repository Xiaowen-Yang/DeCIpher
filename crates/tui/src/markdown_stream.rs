//! MarkdownStreamCollector — newline-gated incremental markdown rendering.
//!
//! Accumulates raw markdown deltas from the LLM stream. When a newline
//! boundary is reached, re-renders the entire buffer through the markdown
//! parser and returns only the NEW lines (delta since last commit).
//!
//! This ensures consistent markdown parsing across chunk boundaries
//! (e.g., a code fence split across two deltas) while keeping the
//! streaming cost low (only delta lines sent to the UI).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Collects streaming markdown deltas and produces rendered lines.
#[derive(Debug)]
pub struct MarkdownStreamCollector {
    /// Raw markdown text accumulated so far.
    buffer: String,
    /// Number of rendered lines already committed.
    committed_line_count: usize,
    /// Terminal width for wrapping decisions.
    width: Option<u16>,
}

impl MarkdownStreamCollector {
    pub fn new(width: Option<u16>) -> Self {
        Self {
            buffer: String::new(),
            committed_line_count: 0,
            width,
        }
    }

    /// Set or update the terminal width.
    pub fn set_width(&mut self, width: u16) {
        if self.width == Some(width) {
            return;
        }
        self.width = Some(width);
    }

    /// Append a delta chunk to the buffer.
    pub fn push_delta(&mut self, delta: &str) {
        self.buffer.push_str(delta);
    }

    /// Commit complete lines (up to the last `\n`).
    ///
    /// Re-renders the entire buffer through the markdown renderer, then
    /// returns only the lines after `committed_line_count` (the delta).
    /// Updates `committed_line_count` to include the newly committed lines.
    ///
    /// Returns an empty vec if no newline is found (nothing to commit).
    pub fn commit_complete_lines(&mut self) -> Vec<Line<'static>> {
        // Only commit when there's at least one complete line
        if !self.buffer.contains('\n') {
            return Vec::new();
        }

        // Find the last newline — only render up to there
        let last_nl = self.buffer.rfind('\n').unwrap();
        let committable = &self.buffer[..=last_nl];

        // Render the committable portion
        let all_lines = self.render_to_lines(committable);

        // Extract only the new lines
        let new_lines = if self.committed_line_count < all_lines.len() {
            all_lines[self.committed_line_count..].to_vec()
        } else {
            Vec::new()
        };

        self.committed_line_count = all_lines.len();
        new_lines
    }

    /// Flush everything, including any partial line at the end.
    ///
    /// Called when the stream ends. Returns all remaining uncommitted lines.
    pub fn finalize_and_drain(&mut self) -> Vec<Line<'static>> {
        if self.buffer.is_empty() {
            return Vec::new();
        }

        let all_lines = self.render_to_lines(&self.buffer.clone());

        let new_lines = if self.committed_line_count < all_lines.len() {
            all_lines[self.committed_line_count..].to_vec()
        } else {
            Vec::new()
        };

        // Reset state
        self.buffer.clear();
        self.committed_line_count = 0;

        new_lines
    }

    /// The current partial line (text after the last `\n`).
    /// Used for streaming preview in the viewport.
    pub fn partial_line(&self) -> &str {
        match self.buffer.rfind('\n') {
            Some(pos) => &self.buffer[pos + 1..],
            None => &self.buffer,
        }
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Render markdown text to styled ratatui lines.
    ///
    /// This is a simplified renderer for streaming — it handles basic
    /// markdown formatting (bold, italic, code, headings, lists) and
    /// produces `Line<'static>` values. In Phase 3 this will be replaced
    /// by a ratatui-native version of the full markdown renderer.
    fn render_to_lines(&self, text: &str) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let indent = "  ";

        for raw_line in text.lines() {
            let trimmed = raw_line.trim();

            // Heading
            if trimmed.starts_with("### ") {
                lines.push(Line::from(vec![
                    Span::raw(indent.to_string()),
                    Span::styled(
                        trimmed[4..].to_string(),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                ]));
            } else if trimmed.starts_with("## ") {
                lines.push(Line::from(vec![
                    Span::raw(indent.to_string()),
                    Span::styled(
                        trimmed[3..].to_string(),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                ]));
            } else if trimmed.starts_with("# ") {
                lines.push(Line::from(vec![
                    Span::raw(indent.to_string()),
                    Span::styled(
                        trimmed[2..].to_string(),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            // Bullet list
            else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                lines.push(Line::from(vec![
                    Span::raw(indent.to_string()),
                    Span::styled("\u{2022} ", Style::default().fg(Color::Rgb(232, 163, 23))),
                    Span::raw(trimmed[2..].to_string()),
                ]));
            }
            // Numbered list
            else if trimmed.len() > 2
                && trimmed.as_bytes()[0].is_ascii_digit()
                && trimmed.contains(". ")
            {
                if let Some(dot_pos) = trimmed.find(". ") {
                    let num = &trimmed[..dot_pos + 2];
                    let rest = &trimmed[dot_pos + 2..];
                    lines.push(Line::from(vec![
                        Span::raw(indent.to_string()),
                        Span::styled(num.to_string(), Style::default().add_modifier(Modifier::DIM)),
                        Span::raw(rest.to_string()),
                    ]));
                } else {
                    lines.push(Line::from(format!("{}{}", indent, trimmed)));
                }
            }
            // Code fence markers (skip)
            else if trimmed.starts_with("```") {
                // Don't render fence markers — code block rendering is simplified
                // to just showing lines with code color
            }
            // Blockquote
            else if trimmed.starts_with("> ") {
                lines.push(Line::from(vec![
                    Span::raw(indent.to_string()),
                    Span::styled("\u{2502} ", Style::default().fg(Color::Green)),
                    Span::raw(trimmed[2..].to_string()),
                ]));
            }
            // Horizontal rule
            else if trimmed == "---" || trimmed == "***" || trimmed == "___" {
                let w = self.width.unwrap_or(80) as usize;
                let dashes = w.saturating_sub(indent.len() + 2).max(8);
                lines.push(Line::from(vec![
                    Span::raw(indent.to_string()),
                    Span::styled(
                        "\u{2500}".repeat(dashes),
                        Style::default().add_modifier(Modifier::DIM),
                    ),
                ]));
            }
            // Empty line
            else if trimmed.is_empty() {
                lines.push(Line::from(""));
            }
            // Regular text — apply inline formatting
            else {
                let spans = parse_inline_markdown(trimmed);
                let mut line_spans = vec![Span::raw(indent.to_string())];
                line_spans.extend(spans);
                lines.push(Line::from(line_spans));
            }
        }

        lines
    }
}

/// Parse inline markdown formatting (bold, italic, code) into spans.
fn parse_inline_markdown(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut chars = text.chars().peekable();
    let mut current = String::new();

    while let Some(ch) = chars.next() {
        match ch {
            '`' => {
                // Inline code
                if !current.is_empty() {
                    spans.push(Span::raw(std::mem::take(&mut current)));
                }
                let mut code = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '`' {
                        chars.next();
                        break;
                    }
                    code.push(c);
                    chars.next();
                }
                spans.push(Span::styled(code, Style::default().fg(Color::Green)));
            }
            '*' if chars.peek() == Some(&'*') => {
                // Bold
                chars.next(); // consume second *
                if !current.is_empty() {
                    spans.push(Span::raw(std::mem::take(&mut current)));
                }
                let mut bold_text = String::new();
                while let Some(c) = chars.next() {
                    if c == '*' && chars.peek() == Some(&'*') {
                        chars.next();
                        break;
                    }
                    bold_text.push(c);
                }
                spans.push(Span::styled(
                    bold_text,
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            }
            '*' | '_' => {
                // Italic
                if !current.is_empty() {
                    spans.push(Span::raw(std::mem::take(&mut current)));
                }
                let mut italic_text = String::new();
                while let Some(&c) = chars.peek() {
                    if c == ch {
                        chars.next();
                        break;
                    }
                    italic_text.push(c);
                    chars.next();
                }
                spans.push(Span::styled(
                    italic_text,
                    Style::default().add_modifier(Modifier::ITALIC),
                ));
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        spans.push(Span::raw(current));
    }

    if spans.is_empty() {
        spans.push(Span::raw(text.to_string()));
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_commit() {
        let mut collector = MarkdownStreamCollector::new(Some(80));
        collector.push_delta("Hello ");
        assert!(collector.commit_complete_lines().is_empty()); // no newline yet

        collector.push_delta("world\n");
        let lines = collector.commit_complete_lines();
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn multiple_lines() {
        let mut collector = MarkdownStreamCollector::new(Some(80));
        collector.push_delta("line one\nline two\n");
        let lines = collector.commit_complete_lines();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn partial_line_preserved() {
        let mut collector = MarkdownStreamCollector::new(Some(80));
        collector.push_delta("complete\npartial");

        let lines = collector.commit_complete_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(collector.partial_line(), "partial");
    }

    #[test]
    fn finalize_flushes_partial() {
        let mut collector = MarkdownStreamCollector::new(Some(80));
        collector.push_delta("complete\npartial");
        collector.commit_complete_lines();

        let remaining = collector.finalize_and_drain();
        assert_eq!(remaining.len(), 1); // the partial line
        assert!(collector.is_empty());
    }

    #[test]
    fn incremental_commit() {
        let mut collector = MarkdownStreamCollector::new(Some(80));

        collector.push_delta("first\n");
        let l1 = collector.commit_complete_lines();
        assert_eq!(l1.len(), 1);

        collector.push_delta("second\n");
        let l2 = collector.commit_complete_lines();
        assert_eq!(l2.len(), 1); // only the new line
    }

    #[test]
    fn heading_rendering() {
        let mut collector = MarkdownStreamCollector::new(Some(80));
        collector.push_delta("# Hello\n");
        let lines = collector.commit_complete_lines();
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn bullet_list() {
        let mut collector = MarkdownStreamCollector::new(Some(80));
        collector.push_delta("- item one\n- item two\n");
        let lines = collector.commit_complete_lines();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn inline_code() {
        let spans = parse_inline_markdown("use `cargo build` here");
        assert!(spans.len() >= 3); // text + code + text
    }

    #[test]
    fn inline_bold() {
        let spans = parse_inline_markdown("this is **bold** text");
        assert!(spans.len() >= 3);
    }

    #[test]
    fn empty_buffer() {
        let mut collector = MarkdownStreamCollector::new(Some(80));
        assert!(collector.commit_complete_lines().is_empty());
        assert!(collector.finalize_and_drain().is_empty());
    }
}
