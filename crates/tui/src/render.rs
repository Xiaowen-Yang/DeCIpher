//! Terminal rendering utilities.
//!
//! After the Phase 2 migration to ratatui inline viewport, this module
//! contains only utility functions (notifications, terminal title) that
//! operate outside the ratatui buffer system.
//!
//! The main rendering is handled by:
//! - `bottom_pane.rs` — viewport widget (prompt, spinner, hints, popups)
//! - `terminal.insert_before()` — permanent scrollback
//! - `pager.rs` — transcript overlay

use std::io::{self, Write};

/// Send a desktop notification.
/// Uses OSC 9 on supported terminals (iTerm2, WezTerm, Kitty),
/// falls back to BEL on others.
pub fn send_notification(o: &mut io::Stdout, message: &str) -> io::Result<()> {
    let clean: String = message.chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    write!(o, "\x1b]9;{clean}\x07")?;
    o.flush()
}

/// Send a BEL (audible bell) notification.
pub fn send_bell(o: &mut io::Stdout) -> io::Result<()> {
    write!(o, "\x07")?;
    o.flush()
}

/// Set terminal window title via OSC 0.
pub fn set_terminal_title(o: &mut io::Stdout, title: &str) -> io::Result<()> {
    let clean: String = title.chars()
        .filter(|c| !c.is_control())
        .take(240)
        .collect();
    write!(o, "\x1b]0;{clean}\x07")?;
    o.flush()
}
