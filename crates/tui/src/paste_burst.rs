//! Paste burst detection for non-bracketed terminals.
//!
//! When a terminal doesn't support bracketed paste, rapid character input
//! (e.g., pasting) looks like very fast typing. This module detects such
//! bursts and buffers them to avoid treating Enter as "submit" mid-paste.
//!
//! Codex ref: `codex-rs/tui/src/bottom_pane/paste_burst.rs`

use std::time::{Duration, Instant};

/// Inter-character timeout for burst detection.
#[cfg(not(windows))]
const BURST_INTERVAL: Duration = Duration::from_millis(8);
#[cfg(windows)]
const BURST_INTERVAL: Duration = Duration::from_millis(30);

/// Idle timeout while burst is active (flush if no new chars arrive).
#[cfg(not(windows))]
const ACTIVE_IDLE_TIMEOUT: Duration = Duration::from_millis(8);
#[cfg(windows)]
const ACTIVE_IDLE_TIMEOUT: Duration = Duration::from_millis(60);

/// Minimum consecutive chars to trigger burst detection.
const MIN_CHARS: u16 = 3;

/// After burst flush, Enter is treated as newline for this duration.
const ENTER_SUPPRESS_WINDOW: Duration = Duration::from_millis(120);

/// Decision returned to the caller for each character.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CharDecision {
    /// Insert the character normally.
    Insert,
    /// Buffer the character (paste burst active).
    Buffer,
}

/// Whether Enter should be treated as submit or newline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnterDecision {
    /// Normal submit behavior.
    Submit,
    /// Treat as newline (we're in a paste burst or just finished one).
    Newline,
}

pub struct PasteBurst {
    /// Timestamp of last plain character event.
    last_char_time: Option<Instant>,
    /// Consecutive chars within the burst interval.
    consecutive: u16,
    /// Whether burst mode is currently active.
    active: bool,
    /// Buffered text during burst.
    buffer: String,
    /// When the burst ended (for Enter suppression window).
    last_flush_time: Option<Instant>,
}

impl PasteBurst {
    pub fn new() -> Self {
        Self {
            last_char_time: None,
            consecutive: 0,
            active: false,
            buffer: String::new(),
            last_flush_time: None,
        }
    }

    /// Process an incoming character. Returns the decision.
    pub fn on_char(&mut self, ch: char) -> CharDecision {
        let now = Instant::now();

        let within_burst = match self.last_char_time {
            Some(last) => now.duration_since(last) <= BURST_INTERVAL,
            None => false,
        };

        self.last_char_time = Some(now);

        if within_burst {
            self.consecutive += 1;
        } else {
            self.consecutive = 1;
        }

        if self.active {
            self.buffer.push(ch);
            return CharDecision::Buffer;
        }

        if self.consecutive >= MIN_CHARS {
            self.active = true;
            self.buffer.push(ch);
            CharDecision::Buffer
        } else {
            CharDecision::Insert
        }
    }

    /// Should Enter be treated as submit or newline?
    pub fn enter_decision(&self) -> EnterDecision {
        if self.active {
            return EnterDecision::Newline;
        }
        if let Some(flush_time) = self.last_flush_time {
            if Instant::now().duration_since(flush_time) <= ENTER_SUPPRESS_WINDOW {
                return EnterDecision::Newline;
            }
        }
        EnterDecision::Submit
    }

    /// Check if the buffer should be flushed (idle timeout expired).
    /// Returns buffered content if ready to flush.
    pub fn flush_if_due(&mut self) -> Option<String> {
        if !self.active {
            return None;
        }
        let now = Instant::now();
        match self.last_char_time {
            Some(last) if now.duration_since(last) > ACTIVE_IDLE_TIMEOUT => {
                self.active = false;
                self.consecutive = 0;
                self.last_flush_time = Some(now);
                if self.buffer.is_empty() {
                    None
                } else {
                    Some(std::mem::take(&mut self.buffer))
                }
            }
            _ => None,
        }
    }

    /// Whether burst mode is currently active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Force flush (e.g., on Enter during burst).
    pub fn force_flush(&mut self) -> Option<String> {
        if self.buffer.is_empty() {
            return None;
        }
        self.active = false;
        self.consecutive = 0;
        self.last_flush_time = Some(Instant::now());
        Some(std::mem::take(&mut self.buffer))
    }
}
