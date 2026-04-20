//! ANSI escape code parser for command output rendering.
//!
//! Parses ANSI SGR (Select Graphic Rendition) codes and converts them
//! to crossterm styling. Handles colors (16, 256, truecolor), bold,
//! italic, underline, dim, and reset sequences.
//!
//! Codex ref: `codex-rs/ansi-escape/src/lib.rs`

use std::io;
use crossterm::style::{Attribute, Color};

const TAB_SPACES: &str = "    ";

/// A span of text with associated ANSI style.
#[derive(Debug, Clone)]
pub struct StyledSpan {
    pub text: String,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl StyledSpan {
    fn new() -> Self {
        Self {
            text: String::new(),
            fg: None,
            bg: None,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            strikethrough: false,
        }
    }

    fn from_state(state: &AnsiState) -> Self {
        Self {
            text: String::new(),
            fg: state.fg,
            bg: state.bg,
            bold: state.bold,
            dim: state.dim,
            italic: state.italic,
            underline: state.underline,
            strikethrough: state.strikethrough,
        }
    }

    /// Convert style attributes to crossterm Attributes.
    pub fn attributes(&self) -> Vec<Attribute> {
        let mut attrs = Vec::new();
        if self.bold { attrs.push(Attribute::Bold); }
        if self.dim { attrs.push(Attribute::Dim); }
        if self.italic { attrs.push(Attribute::Italic); }
        if self.underline { attrs.push(Attribute::Underlined); }
        if self.strikethrough { attrs.push(Attribute::CrossedOut); }
        attrs
    }
}

#[derive(Debug, Clone, Default)]
struct AnsiState {
    fg: Option<Color>,
    bg: Option<Color>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
}

impl AnsiState {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Parse ANSI-escaped text into styled spans.
/// Also expands tabs to 4 spaces.
pub fn parse_ansi(input: &str) -> Vec<StyledSpan> {
    let input = input.replace('\t', TAB_SPACES);
    let mut spans = Vec::new();
    let mut state = AnsiState::default();
    let mut current = StyledSpan::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Check for CSI sequence: ESC [
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                let mut params = String::new();
                // Read until we hit a letter (the command)
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                    params.push(c);
                    chars.next();
                }
                if let Some(&cmd) = chars.peek() {
                    chars.next();
                    if cmd == 'm' {
                        // SGR sequence — flush current span and apply
                        if !current.text.is_empty() {
                            spans.push(current);
                        }
                        apply_sgr(&mut state, &params);
                        current = StyledSpan::from_state(&state);
                    }
                    // Skip other CSI sequences (cursor moves, etc.)
                }
            }
            // Skip other escape sequences (OSC, etc.)
        } else {
            current.text.push(ch);
        }
    }

    if !current.text.is_empty() {
        spans.push(current);
    }

    spans
}

fn apply_sgr(state: &mut AnsiState, params: &str) {
    if params.is_empty() {
        state.reset();
        return;
    }

    let codes: Vec<u32> = params.split(';')
        .filter_map(|s| s.parse().ok())
        .collect();

    let mut i = 0;
    while i < codes.len() {
        match codes[i] {
            0 => state.reset(),
            1 => state.bold = true,
            2 => state.dim = true,
            3 => state.italic = true,
            4 => state.underline = true,
            9 => state.strikethrough = true,
            22 => { state.bold = false; state.dim = false; }
            23 => state.italic = false,
            24 => state.underline = false,
            29 => state.strikethrough = false,
            // Standard foreground colors
            30..=37 => state.fg = Some(ansi_4bit_color(codes[i] - 30)),
            39 => state.fg = None,
            // Standard background colors
            40..=47 => state.bg = Some(ansi_4bit_color(codes[i] - 40)),
            49 => state.bg = None,
            // Bright foreground
            90..=97 => state.fg = Some(ansi_4bit_bright(codes[i] - 90)),
            // Bright background
            100..=107 => state.bg = Some(ansi_4bit_bright(codes[i] - 100)),
            // Extended color: 38;5;N (256-color) or 38;2;R;G;B (truecolor)
            38 => {
                if i + 1 < codes.len() {
                    match codes[i + 1] {
                        5 if i + 2 < codes.len() => {
                            state.fg = Some(Color::AnsiValue(codes[i + 2] as u8));
                            i += 2;
                        }
                        2 if i + 4 < codes.len() => {
                            state.fg = Some(Color::Rgb {
                                r: codes[i + 2] as u8,
                                g: codes[i + 3] as u8,
                                b: codes[i + 4] as u8,
                            });
                            i += 4;
                        }
                        _ => { i += 1; }
                    }
                }
            }
            48 => {
                if i + 1 < codes.len() {
                    match codes[i + 1] {
                        5 if i + 2 < codes.len() => {
                            state.bg = Some(Color::AnsiValue(codes[i + 2] as u8));
                            i += 2;
                        }
                        2 if i + 4 < codes.len() => {
                            state.bg = Some(Color::Rgb {
                                r: codes[i + 2] as u8,
                                g: codes[i + 3] as u8,
                                b: codes[i + 4] as u8,
                            });
                            i += 4;
                        }
                        _ => { i += 1; }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
}

fn ansi_4bit_color(code: u32) -> Color {
    match code {
        0 => Color::Black,
        1 => Color::DarkRed,
        2 => Color::DarkGreen,
        3 => Color::DarkYellow,
        4 => Color::DarkBlue,
        5 => Color::DarkMagenta,
        6 => Color::DarkCyan,
        7 => Color::Grey,
        _ => Color::Reset,
    }
}

fn ansi_4bit_bright(code: u32) -> Color {
    match code {
        0 => Color::DarkGrey,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::White,
        _ => Color::Reset,
    }
}

/// Render styled spans to a crossterm writer with proper styling.
pub fn render_styled_spans(o: &mut impl std::io::Write, spans: &[StyledSpan]) -> io::Result<()> {
    use crossterm::queue;
    use crossterm::style::{Print, SetForegroundColor, SetBackgroundColor, SetAttribute, ResetColor};

    for span in spans {
        // Set attributes
        for attr in span.attributes() {
            queue!(o, SetAttribute(attr))?;
        }
        if let Some(fg) = span.fg {
            queue!(o, SetForegroundColor(fg))?;
        }
        if let Some(bg) = span.bg {
            queue!(o, SetBackgroundColor(bg))?;
        }
        queue!(o, Print(&span.text))?;
        queue!(o, ResetColor, SetAttribute(Attribute::Reset))?;
    }
    Ok(())
}
