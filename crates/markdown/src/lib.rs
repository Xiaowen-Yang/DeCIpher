//! Markdown-to-ANSI renderer using pulldown-cmark + syntect.
//!
//! Features:
//! - **bold**, *italic*, ~~strikethrough~~, `inline code`
//! - ```code blocks``` with syntax highlighting
//! - Bullet lists (-, *, +) with nested indentation
//! - Numbered lists with dim markers
//! - Headers (h1-h6) bold cyan
//! - Blockquotes with green `│ ` prefix
//! - Horizontal rules
//! - Links rendered as cyan underlined text
//! - Tables with box-drawing borders

use std::io::{self, Write};
use std::sync::OnceLock;

use crossterm::{
    queue,
    style::{Attribute, Color, Print, SetAttribute, SetForegroundColor, ResetColor},
};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, CodeBlockKind, Alignment};
use syntect::highlighting::{ThemeSet, Style as SynStyle};
use syntect::parsing::SyntaxSet;
use syntect::easy::HighlightLines;

/// Shiba Inu fur color — warm orange-yellow (#E8A317).
pub const SHIBA: Color = Color::Rgb { r: 232, g: 163, b: 23 };

// ── Syntax highlighting singletons ──────────────────────────────────────────

fn syntax_set() -> &'static SyntaxSet {
    static SS: OnceLock<SyntaxSet> = OnceLock::new();
    SS.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static TS: OnceLock<ThemeSet> = OnceLock::new();
    TS.get_or_init(ThemeSet::load_defaults)
}

/// Render a markdown string to the terminal with ANSI styling.
/// Each line is indented by `indent` spaces.
pub fn render_markdown(o: &mut io::Stdout, text: &str, indent: usize) -> io::Result<()> {
    let prefix = " ".repeat(indent);
    let opts = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(text, opts);

    let mut state = RenderState {
        prefix: &prefix,
        bold: false,
        italic: false,
        strikethrough: false,
        in_code_block: false,
        in_heading: false,
        in_blockquote: false,
        in_link: false,
        link_url: String::new(),
        list_stack: Vec::new(),
        code_lang: None,
        code_buf: String::new(),
        line_started: false,
        // Table state
        in_table: false,
        table_alignments: Vec::new(),
        table_row: Vec::new(),
        table_cell_buf: String::new(),
        in_table_head: false,
        table_rows: Vec::new(),
    };

    for event in parser {
        match event {
            Event::Start(tag) => handle_start(o, &mut state, &tag)?,
            Event::End(tag_end) => handle_end(o, &mut state, &tag_end)?,
            Event::Text(text) => handle_text(o, &mut state, &text)?,
            Event::Code(code) => {
                if state.in_table {
                    state.table_cell_buf.push('`');
                    state.table_cell_buf.push_str(&code);
                    state.table_cell_buf.push('`');
                } else {
                    ensure_line_start(o, &state)?;
                    queue!(o, SetForegroundColor(Color::Green), Print(&*code), ResetColor)?;
                }
            }
            Event::SoftBreak => {
                if state.in_heading { queue!(o, Print(" "))?; }
                else if !state.in_table {
                    queue!(o, Print("\r\n"))?;
                    state.line_started = false;
                }
            }
            Event::HardBreak => {
                if !state.in_table {
                    queue!(o, Print("\r\n"))?;
                    state.line_started = false;
                }
            }
            Event::Rule => {
                let w = crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
                let dashes = w.saturating_sub(indent + 2).max(8);
                queue!(o, Print(&state.prefix), SetAttribute(Attribute::Dim),
                    Print("─".repeat(dashes)), SetAttribute(Attribute::Reset), Print("\r\n"))?;
                state.line_started = false;
            }
            _ => {}
        }
    }

    o.flush()
}

struct RenderState<'a> {
    prefix: &'a str,
    bold: bool,
    italic: bool,
    strikethrough: bool,
    in_code_block: bool,
    in_heading: bool,
    in_blockquote: bool,
    in_link: bool,
    link_url: String,
    list_stack: Vec<Option<u64>>, // None = unordered, Some(n) = ordered at n
    code_lang: Option<String>,
    code_buf: String,
    line_started: bool,
    // Table state
    in_table: bool,
    table_alignments: Vec<Alignment>,
    table_row: Vec<String>,
    table_cell_buf: String,
    in_table_head: bool,
    table_rows: Vec<Vec<String>>,
}

fn ensure_line_start(o: &mut impl Write, state: &RenderState) -> io::Result<()> {
    if !state.line_started {
        queue!(o, Print(state.prefix))?;
        if state.in_blockquote {
            queue!(o, SetForegroundColor(Color::Green), Print("│ "), ResetColor)?;
        }
        // List indentation
        let depth = state.list_stack.len();
        if depth > 0 {
            let indent = "  ".repeat(depth);
            queue!(o, Print(indent))?;
        }
    }
    Ok(())
}

fn handle_start(o: &mut impl Write, state: &mut RenderState, tag: &Tag) -> io::Result<()> {
    match tag {
        Tag::Heading { level, .. } => {
            state.in_heading = true;
            ensure_line_start(o, state)?;
            state.line_started = true;
            queue!(o, SetAttribute(Attribute::Bold), SetForegroundColor(Color::Cyan))?;
            let prefix = match *level {
                pulldown_cmark::HeadingLevel::H1 => "# ",
                pulldown_cmark::HeadingLevel::H2 => "## ",
                pulldown_cmark::HeadingLevel::H3 => "### ",
                _ => "",
            };
            if !prefix.is_empty() { queue!(o, Print(prefix))?; }
        }
        Tag::Paragraph => {}
        Tag::BlockQuote(_) => {
            state.in_blockquote = true;
        }
        Tag::List(start) => {
            state.list_stack.push(*start);
        }
        Tag::Item => {
            ensure_line_start(o, state)?;
            state.line_started = true;
            match state.list_stack.last_mut() {
                Some(Some(n)) => {
                    queue!(o, SetAttribute(Attribute::Dim), Print(&format!("{}. ", n)),
                        SetAttribute(Attribute::Reset))?;
                    *n += 1;
                }
                Some(None) => {
                    queue!(o, SetForegroundColor(SHIBA), Print("• "), ResetColor)?;
                }
                None => {}
            }
        }
        Tag::CodeBlock(kind) => {
            state.in_code_block = true;
            state.code_lang = match kind {
                CodeBlockKind::Fenced(lang) => {
                    let l = lang.split_once(|c: char| c == ',' || c.is_whitespace())
                        .map(|(first, _)| first)
                        .unwrap_or(lang);
                    if l.is_empty() { None } else { Some(l.to_string()) }
                }
                CodeBlockKind::Indented => None,
            };
            state.code_buf.clear();
        }
        Tag::Emphasis => {
            state.italic = true;
            if !state.in_table {
                queue!(o, SetAttribute(Attribute::Italic))?;
            }
        }
        Tag::Strong => {
            state.bold = true;
            if !state.in_table {
                queue!(o, SetAttribute(Attribute::Bold))?;
            }
        }
        Tag::Strikethrough => {
            state.strikethrough = true;
            if !state.in_table {
                queue!(o, SetAttribute(Attribute::CrossedOut))?;
            }
        }
        Tag::Link { dest_url, .. } => {
            state.in_link = true;
            state.link_url = dest_url.to_string();
            if !state.in_table {
                queue!(o, SetForegroundColor(Color::Cyan), SetAttribute(Attribute::Underlined))?;
            }
        }
        Tag::Table(alignments) => {
            state.in_table = true;
            state.table_alignments = alignments.clone();
            state.table_rows.clear();
            state.in_table_head = false;
        }
        Tag::TableHead => {
            state.in_table_head = true;
            state.table_row.clear();
        }
        Tag::TableRow => {
            state.table_row.clear();
        }
        Tag::TableCell => {
            state.table_cell_buf.clear();
        }
        _ => {}
    }
    Ok(())
}

fn handle_end(o: &mut impl Write, state: &mut RenderState, tag_end: &TagEnd) -> io::Result<()> {
    match tag_end {
        TagEnd::Heading(_) => {
            state.in_heading = false;
            queue!(o, ResetColor, SetAttribute(Attribute::Reset), Print("\r\n"))?;
            state.line_started = false;
        }
        TagEnd::Paragraph => {
            if !state.in_table {
                queue!(o, Print("\r\n"))?;
                state.line_started = false;
            }
        }
        TagEnd::BlockQuote(_) => {
            state.in_blockquote = false;
        }
        TagEnd::List(_) => {
            state.list_stack.pop();
        }
        TagEnd::Item => {
            if state.line_started {
                queue!(o, Print("\r\n"))?;
                state.line_started = false;
            }
        }
        TagEnd::CodeBlock => {
            render_code_block(o, state)?;
            state.in_code_block = false;
            state.code_lang = None;
            state.code_buf.clear();
        }
        TagEnd::Emphasis => {
            state.italic = false;
            if !state.in_table {
                queue!(o, SetAttribute(Attribute::Reset))?;
                if state.bold { queue!(o, SetAttribute(Attribute::Bold))?; }
            }
        }
        TagEnd::Strong => {
            state.bold = false;
            if !state.in_table {
                queue!(o, SetAttribute(Attribute::Reset))?;
                if state.italic { queue!(o, SetAttribute(Attribute::Italic))?; }
            }
        }
        TagEnd::Strikethrough => {
            state.strikethrough = false;
            if !state.in_table {
                queue!(o, SetAttribute(Attribute::Reset))?;
            }
        }
        TagEnd::Link => {
            state.in_link = false;
            if !state.in_table {
                queue!(o, ResetColor, SetAttribute(Attribute::Reset))?;
            }
        }
        TagEnd::TableCell => {
            state.table_row.push(std::mem::take(&mut state.table_cell_buf));
        }
        TagEnd::TableHead => {
            state.in_table_head = false;
            state.table_rows.insert(0, state.table_row.clone());
            state.table_row.clear();
        }
        TagEnd::TableRow => {
            if !state.in_table_head {
                state.table_rows.push(state.table_row.clone());
            }
            state.table_row.clear();
        }
        TagEnd::Table => {
            render_table(o, state)?;
            state.in_table = false;
            state.table_rows.clear();
            state.table_alignments.clear();
        }
        _ => {}
    }
    Ok(())
}

fn handle_text(o: &mut impl Write, state: &mut RenderState, text: &str) -> io::Result<()> {
    if state.in_code_block {
        state.code_buf.push_str(text);
        return Ok(());
    }

    if state.in_table {
        state.table_cell_buf.push_str(text);
        return Ok(());
    }

    ensure_line_start(o, state)?;
    state.line_started = true;
    queue!(o, Print(text))?;
    Ok(())
}

fn render_code_block(o: &mut impl Write, state: &RenderState) -> io::Result<()> {
    let code = &state.code_buf;
    if code.is_empty() { return Ok(()); }

    let ss = syntax_set();
    let ts = theme_set();

    if let Some(ref lang) = state.code_lang {
        let syntax = ss.find_syntax_by_token(lang)
            .unwrap_or_else(|| ss.find_syntax_plain_text());
        let theme = &ts.themes["base16-ocean.dark"];
        let mut h = HighlightLines::new(syntax, theme);

        for line in code.lines() {
            queue!(o, Print(state.prefix), Print("  "))?;
            if let Ok(ranges) = h.highlight_line(line, ss) {
                for (style, text) in ranges {
                    let fg = syn_color_to_crossterm(style);
                    queue!(o, SetForegroundColor(fg), Print(text), ResetColor)?;
                }
            } else {
                queue!(o, SetForegroundColor(Color::Green), Print(line), ResetColor)?;
            }
            queue!(o, Print("\r\n"))?;
        }
    } else {
        for line in code.lines() {
            queue!(o, Print(state.prefix), Print("  "),
                SetForegroundColor(Color::Green), Print(line), ResetColor, Print("\r\n"))?;
        }
    }

    Ok(())
}

/// Render a table with box-drawing characters.
fn render_table(o: &mut impl Write, state: &RenderState) -> io::Result<()> {
    if state.table_rows.is_empty() { return Ok(()); }

    // Calculate column widths
    let num_cols = state.table_rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut col_widths = vec![0usize; num_cols];
    for row in &state.table_rows {
        for (i, cell) in row.iter().enumerate() {
            if i < num_cols {
                col_widths[i] = col_widths[i].max(cell.chars().count());
            }
        }
    }

    // Top border
    queue!(o, Print(state.prefix), SetAttribute(Attribute::Dim))?;
    queue!(o, Print("┌"))?;
    for (i, &w) in col_widths.iter().enumerate() {
        queue!(o, Print("─".repeat(w + 2)))?;
        if i < num_cols - 1 { queue!(o, Print("┬"))?; }
    }
    queue!(o, Print("┐"), SetAttribute(Attribute::Reset), Print("\r\n"))?;

    for (row_idx, row) in state.table_rows.iter().enumerate() {
        // Data row
        queue!(o, Print(state.prefix), SetAttribute(Attribute::Dim), Print("│"), SetAttribute(Attribute::Reset))?;
        for (i, cell) in row.iter().enumerate() {
            let w = col_widths.get(i).copied().unwrap_or(0);
            let aligned = align_cell(cell, w, state.table_alignments.get(i).copied());

            if row_idx == 0 {
                // Header row: bold
                queue!(o, Print(" "), SetAttribute(Attribute::Bold), Print(&aligned), SetAttribute(Attribute::Reset), Print(" "))?;
            } else {
                queue!(o, Print(" "), Print(&aligned), Print(" "))?;
            }
            queue!(o, SetAttribute(Attribute::Dim), Print("│"), SetAttribute(Attribute::Reset))?;
        }
        // Fill missing columns
        for i in row.len()..num_cols {
            let w = col_widths.get(i).copied().unwrap_or(0);
            queue!(o, Print(" "), Print(&" ".repeat(w)), Print(" "))?;
            queue!(o, SetAttribute(Attribute::Dim), Print("│"), SetAttribute(Attribute::Reset))?;
        }
        queue!(o, Print("\r\n"))?;

        // Separator after header
        if row_idx == 0 {
            queue!(o, Print(state.prefix), SetAttribute(Attribute::Dim))?;
            queue!(o, Print("├"))?;
            for (i, &w) in col_widths.iter().enumerate() {
                queue!(o, Print("─".repeat(w + 2)))?;
                if i < num_cols - 1 { queue!(o, Print("┼"))?; }
            }
            queue!(o, Print("┤"), SetAttribute(Attribute::Reset), Print("\r\n"))?;
        }
    }

    // Bottom border
    queue!(o, Print(state.prefix), SetAttribute(Attribute::Dim))?;
    queue!(o, Print("└"))?;
    for (i, &w) in col_widths.iter().enumerate() {
        queue!(o, Print("─".repeat(w + 2)))?;
        if i < num_cols - 1 { queue!(o, Print("┴"))?; }
    }
    queue!(o, Print("┘"), SetAttribute(Attribute::Reset), Print("\r\n"))?;

    Ok(())
}

fn align_cell(text: &str, width: usize, alignment: Option<Alignment>) -> String {
    let text_len = text.chars().count();
    if text_len >= width { return text.to_string(); }
    let pad = width - text_len;
    match alignment {
        Some(Alignment::Center) => {
            let left = pad / 2;
            let right = pad - left;
            format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
        }
        Some(Alignment::Right) => format!("{}{}", " ".repeat(pad), text),
        _ => format!("{}{}", text, " ".repeat(pad)),
    }
}

fn syn_color_to_crossterm(style: SynStyle) -> Color {
    let fg = style.foreground;
    Color::Rgb { r: fg.r, g: fg.g, b: fg.b }
}
