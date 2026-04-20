//! Terminal capability detection.
//!
//! Detects terminal emulator type, color support level, and
//! background lightness for theme selection.
//!
//! Codex ref: `codex-rs/tui/src/terminal_palette.rs`

use std::env;

/// Terminal emulator type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TerminalType {
    ITerm2,
    WezTerm,
    Kitty,
    Alacritty,
    Ghostty,
    WindowsTerminal,
    VsCode,
    Tmux,
    Screen,
    Unknown,
}

/// Color support level.
#[derive(Debug, Clone, Copy, PartialEq, Ord, PartialOrd, Eq)]
pub enum ColorLevel {
    /// No color support.
    None,
    /// 16 basic colors.
    Ansi16,
    /// 256 indexed colors.
    Ansi256,
    /// Full 24-bit truecolor.
    TrueColor,
}

/// Background tone for theme selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundTone {
    Dark,
    Light,
    Unknown,
}

/// Detected terminal capabilities.
#[derive(Debug, Clone)]
pub struct TerminalCaps {
    pub terminal_type: TerminalType,
    pub color_level: ColorLevel,
    pub background: BackgroundTone,
    pub supports_osc9: bool,
    pub supports_osc8_links: bool,
    pub supports_kitty_keyboard: bool,
    pub supports_sixel: bool,
    pub supports_kitty_graphics: bool,
}

/// Detect terminal capabilities from environment.
pub fn detect() -> TerminalCaps {
    let terminal_type = detect_terminal_type();
    let color_level = detect_color_level(&terminal_type);
    let background = detect_background();

    let supports_osc9 = matches!(
        terminal_type,
        TerminalType::ITerm2 | TerminalType::WezTerm | TerminalType::Kitty
    );

    let supports_osc8_links = matches!(
        terminal_type,
        TerminalType::ITerm2 | TerminalType::WezTerm | TerminalType::Kitty
            | TerminalType::Ghostty | TerminalType::Alacritty
    ) || color_level >= ColorLevel::TrueColor;

    let supports_kitty_keyboard = matches!(
        terminal_type,
        TerminalType::Kitty | TerminalType::WezTerm | TerminalType::Ghostty
    );

    let supports_sixel = matches!(terminal_type, TerminalType::WezTerm);

    let supports_kitty_graphics = matches!(
        terminal_type,
        TerminalType::Kitty | TerminalType::WezTerm
    );

    TerminalCaps {
        terminal_type,
        color_level,
        background,
        supports_osc9,
        supports_osc8_links,
        supports_kitty_keyboard,
        supports_sixel,
        supports_kitty_graphics,
    }
}

fn detect_terminal_type() -> TerminalType {
    // Check TERM_PROGRAM first (most reliable)
    if let Ok(prog) = env::var("TERM_PROGRAM") {
        match prog.to_lowercase().as_str() {
            "iterm.app" | "iterm2" => return TerminalType::ITerm2,
            "wezterm" => return TerminalType::WezTerm,
            "ghostty" => return TerminalType::Ghostty,
            "alacritty" => return TerminalType::Alacritty,
            "vscode" => return TerminalType::VsCode,
            _ => {}
        }
    }

    // Check for Kitty
    if env::var("KITTY_WINDOW_ID").is_ok() {
        return TerminalType::Kitty;
    }

    // Check for Windows Terminal
    if env::var("WT_SESSION").is_ok() {
        return TerminalType::WindowsTerminal;
    }

    // Check for tmux/screen
    if let Ok(term) = env::var("TERM") {
        if term.starts_with("tmux") || term.starts_with("screen") {
            if env::var("TMUX").is_ok() {
                return TerminalType::Tmux;
            }
            return TerminalType::Screen;
        }
    }

    TerminalType::Unknown
}

fn detect_color_level(terminal: &TerminalType) -> ColorLevel {
    // COLORTERM=truecolor is the standard indicator
    if let Ok(ct) = env::var("COLORTERM") {
        match ct.to_lowercase().as_str() {
            "truecolor" | "24bit" => return ColorLevel::TrueColor,
            _ => {}
        }
    }

    // Known truecolor terminals
    match terminal {
        TerminalType::ITerm2
        | TerminalType::WezTerm
        | TerminalType::Kitty
        | TerminalType::Alacritty
        | TerminalType::Ghostty
        | TerminalType::WindowsTerminal
        | TerminalType::VsCode => return ColorLevel::TrueColor,
        _ => {}
    }

    // Check TERM for 256-color
    if let Ok(term) = env::var("TERM") {
        if term.contains("256color") {
            return ColorLevel::Ansi256;
        }
    }

    // Default to 256 on modern systems
    if env::var("TERM").is_ok() {
        ColorLevel::Ansi256
    } else {
        ColorLevel::Ansi16
    }
}

fn detect_background() -> BackgroundTone {
    // COLORFGBG is set by some terminals: "fg;bg" where bg>7 = light
    if let Ok(fgbg) = env::var("COLORFGBG") {
        if let Some(bg_str) = fgbg.split(';').last() {
            if let Ok(bg) = bg_str.parse::<u32>() {
                return if bg > 7 { BackgroundTone::Light } else { BackgroundTone::Dark };
            }
        }
    }

    // Most terminals default to dark
    BackgroundTone::Dark
}
