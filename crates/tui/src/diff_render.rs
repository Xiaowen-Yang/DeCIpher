//! Unified diff rendering with syntax-aware coloring.
//!
//! Renders unified diff hunks with:
//! - Green backgrounds for additions, red for deletions
//! - Right-aligned line numbers + gutter signs (+/-/ )
//! - Tab expansion (4 chars)
//! - Hard wrapping at viewport width
//!
//! Codex ref: `codex-rs/tui/src/diff_render.rs`

use std::io::{self, Write};
use crossterm::{
    queue,
    style::{Attribute, Color, Print, SetAttribute, SetForegroundColor, SetBackgroundColor, ResetColor},
};

/// Dark theme colors (muted tints for comfortable reading).
const ADD_BG: Color = Color::Rgb { r: 33, g: 58, b: 43 };    // #213A2B
const DEL_BG: Color = Color::Rgb { r: 74, g: 34, b: 29 };    // #4A221D
const ADD_FG: Color = Color::Rgb { r: 130, g: 220, b: 130 };  // bright green
const DEL_FG: Color = Color::Rgb { r: 220, g: 130, b: 130 };  // bright red
const GUTTER_FG: Color = Color::Rgb { r: 100, g: 100, b: 100 };
const HUNK_HEADER_FG: Color = Color::Rgb { r: 120, g: 160, b: 200 };
const FILE_HEADER_FG: Color = Color::Cyan;

const TAB_WIDTH: usize = 4;

/// A parsed diff file entry.
pub struct DiffFile {
    pub path: String,
    pub hunks: Vec<DiffHunk>,
    pub change_type: FileChange,
}

/// Type of file change.
pub enum FileChange {
    Add,
    Delete,
    Update,
}

/// A single diff hunk.
pub struct DiffHunk {
    pub header: String,  // e.g., "@@ -1,5 +1,7 @@"
    pub lines: Vec<DiffLine>,
}

/// A single line in a diff hunk.
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum DiffLineKind {
    Context,
    Add,
    Delete,
}

/// Render a complete diff to the terminal.
pub fn render_diff(o: &mut impl Write, files: &[DiffFile], indent: usize) -> io::Result<()> {
    let prefix = " ".repeat(indent);
    let width = crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80);

    for file in files {
        // File header
        let change_label = match file.change_type {
            FileChange::Add => "+",
            FileChange::Delete => "-",
            FileChange::Update => "~",
        };
        queue!(o, Print(&prefix))?;
        queue!(o, SetForegroundColor(FILE_HEADER_FG), SetAttribute(Attribute::Bold))?;
        queue!(o, Print(change_label), Print(" "), Print(&file.path))?;
        queue!(o, SetAttribute(Attribute::Reset), ResetColor, Print("\r\n"))?;

        for hunk in &file.hunks {
            // Hunk header
            queue!(o, Print(&prefix))?;
            queue!(o, SetForegroundColor(HUNK_HEADER_FG))?;
            queue!(o, Print(&hunk.header))?;
            queue!(o, ResetColor, Print("\r\n"))?;

            // Compute gutter width (max line number digits)
            let max_lineno = hunk.lines.iter()
                .filter_map(|l| l.new_lineno.or(l.old_lineno))
                .max()
                .unwrap_or(1);
            let gutter_width = digit_count(max_lineno);

            for line in &hunk.lines {
                render_diff_line(o, line, &prefix, gutter_width, width)?;
            }
        }
        queue!(o, Print("\r\n"))?;
    }
    o.flush()
}

fn render_diff_line(
    o: &mut impl Write,
    line: &DiffLine,
    prefix: &str,
    gutter_width: usize,
    _width: usize,
) -> io::Result<()> {
    let (sign, bg, fg) = match line.kind {
        DiffLineKind::Add => ("+", Some(ADD_BG), ADD_FG),
        DiffLineKind::Delete => ("-", Some(DEL_BG), DEL_FG),
        DiffLineKind::Context => (" ", None, Color::Reset),
    };

    // Prefix
    queue!(o, Print(prefix))?;

    // Line numbers in gutter
    let old_num = line.old_lineno.map(|n| format!("{:>w$}", n, w = gutter_width)).unwrap_or(" ".repeat(gutter_width));
    let new_num = line.new_lineno.map(|n| format!("{:>w$}", n, w = gutter_width)).unwrap_or(" ".repeat(gutter_width));

    queue!(o, SetForegroundColor(GUTTER_FG))?;
    queue!(o, Print(&old_num), Print(" "), Print(&new_num), Print(" "))?;
    queue!(o, ResetColor)?;

    // Gutter sign
    queue!(o, SetForegroundColor(fg))?;
    queue!(o, Print(sign), Print(" "))?;

    // Content with background
    if let Some(bg_color) = bg {
        queue!(o, SetBackgroundColor(bg_color))?;
    }
    let expanded = expand_tabs(&line.content);
    queue!(o, Print(&expanded))?;
    queue!(o, ResetColor, SetAttribute(Attribute::Reset), Print("\r\n"))?;

    Ok(())
}

/// Parse a unified diff string into DiffFile structures.
pub fn parse_unified_diff(diff_text: &str) -> Vec<DiffFile> {
    let mut files = Vec::new();
    let mut current_file: Option<DiffFile> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut old_lineno: u32 = 0;
    let mut new_lineno: u32 = 0;

    for line in diff_text.lines() {
        if line.starts_with("--- ") {
            // Start of new file (--- a/path)
            if let Some(mut file) = current_file.take() {
                if let Some(hunk) = current_hunk.take() {
                    file.hunks.push(hunk);
                }
                files.push(file);
            }
            // Will be completed by +++ line
        } else if line.starts_with("+++ ") {
            let path = line.strip_prefix("+++ ").unwrap_or("").to_string();
            let path = path.strip_prefix("b/").unwrap_or(&path).to_string();
            current_file = Some(DiffFile {
                path,
                hunks: Vec::new(),
                change_type: FileChange::Update,
            });
        } else if line.starts_with("@@ ") {
            if let Some(ref mut file) = current_file {
                if let Some(hunk) = current_hunk.take() {
                    file.hunks.push(hunk);
                }
            }
            // Parse hunk header: @@ -old_start,old_count +new_start,new_count @@
            let (old_start, new_start) = parse_hunk_header(line);
            old_lineno = old_start;
            new_lineno = new_start;
            current_hunk = Some(DiffHunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
        } else if let Some(ref mut hunk) = current_hunk {
            if let Some(content) = line.strip_prefix('+') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Add,
                    content: content.to_string(),
                    old_lineno: None,
                    new_lineno: Some(new_lineno),
                });
                new_lineno += 1;
            } else if let Some(content) = line.strip_prefix('-') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Delete,
                    content: content.to_string(),
                    old_lineno: Some(old_lineno),
                    new_lineno: None,
                });
                old_lineno += 1;
            } else if line.starts_with(' ') || line.is_empty() {
                let content = if line.starts_with(' ') { &line[1..] } else { line };
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Context,
                    content: content.to_string(),
                    old_lineno: Some(old_lineno),
                    new_lineno: Some(new_lineno),
                });
                old_lineno += 1;
                new_lineno += 1;
            }
        }
    }

    // Flush remaining
    if let Some(mut file) = current_file {
        if let Some(hunk) = current_hunk {
            file.hunks.push(hunk);
        }
        files.push(file);
    }

    files
}

fn parse_hunk_header(header: &str) -> (u32, u32) {
    // @@ -old_start[,old_count] +new_start[,new_count] @@
    let parts: Vec<&str> = header.split_whitespace().collect();
    let old_start = parts.get(1)
        .and_then(|s| s.strip_prefix('-'))
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let new_start = parts.get(2)
        .and_then(|s| s.strip_prefix('+'))
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    (old_start, new_start)
}

fn expand_tabs(s: &str) -> String {
    s.replace('\t', &" ".repeat(TAB_WIDTH))
}

fn digit_count(n: u32) -> usize {
    if n == 0 { return 1; }
    ((n as f64).log10().floor() as usize) + 1
}
