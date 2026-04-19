//! Markdown-to-ANSI renderer using pulldown-cmark + syntect.
//!
//! Features:
//! - **bold**, *italic*, ~~strikethrough~~, `inline code`
//! - ```code blocks``` with syntax highlighting
//! - Bullet lists (-, *, +) with nested indentation
//! - Numbered lists with dim markers
//! - Headers (h1-h6) bold cyan
//! - Blockquotes with green `> ` prefix
//! - Horizontal rules
//! - Links rendered as cyan underlined text

use std::io::{self, Write};
use std::sync::OnceLock;

use crossterm::{
    queue,
    style::{Attribute, Color, Print, SetAttribute, SetForegroundColor, ResetColor},
};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, CodeBlockKind};
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
    };

    for event in parser {
        match event {
            Event::Start(tag) => handle_start(o, &mut state, &tag)?,
            Event::End(tag_end) => handle_end(o, &mut state, &tag_end)?,
            Event::Text(text) => handle_text(o, &mut state, &text)?,
            Event::Code(code) => {
                ensure_line_start(o, &state)?;
                queue!(o, SetForegroundColor(Color::Green), Print(&*code), ResetColor)?;
            }
            Event::SoftBreak => {
                if state.in_heading { queue!(o, Print(" "))?; }
                else { queue!(o, Print("\r\n"))?; state.line_started = false; }
            }
            Event::HardBreak => {
                queue!(o, Print("\r\n"))?;
                state.line_started = false;
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
            // Add h-level prefix
            let prefix = match *level {
                pulldown_cmark::HeadingLevel::H1 => "# ",
                pulldown_cmark::HeadingLevel::H2 => "## ",
                pulldown_cmark::HeadingLevel::H3 => "### ",
                _ => "",
            };
            if !prefix.is_empty() { queue!(o, Print(prefix))?; }
        }
        Tag::Paragraph => {
            // Start new paragraph
        }
        Tag::BlockQuote(_) => {
            state.in_blockquote = true;
        }
        Tag::List(start) => {
            state.list_stack.push(*start);
        }
        Tag::Item => {
            ensure_line_start(o, state)?;
            state.line_started = true;
            // Move back one indent level for the bullet/number
            let stack_len = state.list_stack.len();
            if stack_len > 0 {
                // Overwrite the indent we just wrote by going back
            }
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
            queue!(o, SetAttribute(Attribute::Italic))?;
        }
        Tag::Strong => {
            state.bold = true;
            queue!(o, SetAttribute(Attribute::Bold))?;
        }
        Tag::Strikethrough => {
            state.strikethrough = true;
            queue!(o, SetAttribute(Attribute::CrossedOut))?;
        }
        Tag::Link { dest_url, .. } => {
            state.in_link = true;
            state.link_url = dest_url.to_string();
            queue!(o, SetForegroundColor(Color::Cyan), SetAttribute(Attribute::Underlined))?;
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
            queue!(o, Print("\r\n"))?;
            state.line_started = false;
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
            queue!(o, SetAttribute(Attribute::Reset))?;
            // Restore other active styles
            if state.bold { queue!(o, SetAttribute(Attribute::Bold))?; }
        }
        TagEnd::Strong => {
            state.bold = false;
            queue!(o, SetAttribute(Attribute::Reset))?;
            if state.italic { queue!(o, SetAttribute(Attribute::Italic))?; }
        }
        TagEnd::Strikethrough => {
            state.strikethrough = false;
            queue!(o, SetAttribute(Attribute::Reset))?;
        }
        TagEnd::Link => {
            state.in_link = false;
            queue!(o, ResetColor, SetAttribute(Attribute::Reset))?;
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

    // Try syntax highlighting
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
        // No language — render as green
        for line in code.lines() {
            queue!(o, Print(state.prefix), Print("  "),
                SetForegroundColor(Color::Green), Print(line), ResetColor, Print("\r\n"))?;
        }
    }

    Ok(())
}

fn syn_color_to_crossterm(style: SynStyle) -> Color {
    let fg = style.foreground;
    Color::Rgb { r: fg.r, g: fg.g, b: fg.b }
}
