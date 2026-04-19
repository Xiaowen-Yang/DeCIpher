//! Streaming pipeline — newline-gated buffering with adaptive chunking.
//!
//! Inspired by Codex's streaming architecture:
//! - Deltas collected into a line buffer
//! - Complete lines queued with timestamps
//! - Adaptive chunking drains the queue (smooth vs catch-up mode)
//! - Commit tick orchestrates the drain

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// A complete line ready to be rendered.
#[derive(Debug)]
pub struct QueuedLine {
    pub text: String,
    pub enqueued_at: Instant,
}

/// Adaptive chunking mode.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ChunkingMode {
    /// Drain 1 line per tick — smooth pacing for user perception.
    Smooth,
    /// Drain all queued lines — catching up when backlog forms.
    CatchUp,
}

/// How many lines to drain this tick.
#[derive(Debug)]
pub enum DrainPlan {
    /// Drain exactly one line.
    Single,
    /// Drain up to N lines.
    Batch(usize),
    /// Nothing to drain.
    None,
}

/// Thresholds for adaptive chunking (matching Codex defaults).
const ENTER_QUEUE_DEPTH: usize = 8;
const ENTER_OLDEST_AGE: Duration = Duration::from_millis(120);
const EXIT_QUEUE_DEPTH: usize = 2;
const EXIT_OLDEST_AGE: Duration = Duration::from_millis(40);
const _EXIT_HOLD: Duration = Duration::from_millis(250);
const REENTER_HOLD: Duration = Duration::from_millis(250);
const SEVERE_QUEUE_DEPTH: usize = 64;

/// The streaming state machine.
pub struct StreamState {
    /// Partial line buffer — accumulates until newline.
    collector: String,
    /// Queue of complete lines ready to render.
    queue: VecDeque<QueuedLine>,
    /// Current chunking mode.
    mode: ChunkingMode,
    /// When we last switched OUT of catch-up.
    last_exit_catchup: Option<Instant>,
    /// Whether streaming is active.
    pub active: bool,
}

impl StreamState {
    pub fn new() -> Self {
        Self {
            collector: String::new(),
            queue: VecDeque::new(),
            mode: ChunkingMode::Smooth,
            last_exit_catchup: None,
            active: false,
        }
    }

    /// Push a delta chunk. Complete lines (ending with \n) are moved to the queue.
    pub fn push(&mut self, delta: &str) {
        self.active = true;
        self.collector.push_str(delta);

        // Commit complete lines to the queue
        while let Some(nl_pos) = self.collector.find('\n') {
            let line = self.collector[..nl_pos].to_string();
            self.collector = self.collector[nl_pos + 1..].to_string();
            self.queue.push_back(QueuedLine {
                text: line,
                enqueued_at: Instant::now(),
            });
        }
    }

    /// Get the partial (uncommitted) line content for preview.
    pub fn partial_line(&self) -> &str {
        &self.collector
    }

    /// Number of queued complete lines.
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Run the commit tick — decide how many lines to drain.
    pub fn commit_tick(&mut self) -> DrainPlan {
        if self.queue.is_empty() {
            return DrainPlan::None;
        }

        let now = Instant::now();
        let queue_depth = self.queue.len();
        let oldest_age = now.duration_since(self.queue.front().unwrap().enqueued_at);

        // Decide mode transition
        match self.mode {
            ChunkingMode::Smooth => {
                // Enter catch-up if queue pressure is high
                if queue_depth >= SEVERE_QUEUE_DEPTH
                    || queue_depth >= ENTER_QUEUE_DEPTH
                    || oldest_age >= ENTER_OLDEST_AGE
                {
                    // Check reentry cooldown
                    let can_enter = match self.last_exit_catchup {
                        Some(t) if queue_depth < SEVERE_QUEUE_DEPTH =>
                            now.duration_since(t) >= REENTER_HOLD,
                        _ => true,
                    };
                    if can_enter {
                        self.mode = ChunkingMode::CatchUp;
                        DrainPlan::Batch(queue_depth)
                    } else {
                        DrainPlan::Single
                    }
                } else {
                    DrainPlan::Single
                }
            }
            ChunkingMode::CatchUp => {
                // Exit catch-up if pressure is low enough
                if queue_depth <= EXIT_QUEUE_DEPTH && oldest_age <= EXIT_OLDEST_AGE {
                    // Hold in catch-up for EXIT_HOLD before switching
                    self.mode = ChunkingMode::Smooth;
                    self.last_exit_catchup = Some(now);
                    DrainPlan::Single
                } else {
                    DrainPlan::Batch(queue_depth)
                }
            }
        }
    }

    /// Drain up to `n` lines from the front of the queue.
    pub fn drain(&mut self, n: usize) -> Vec<String> {
        let take = n.min(self.queue.len());
        let mut lines = Vec::with_capacity(take);
        for _ in 0..take {
            if let Some(ql) = self.queue.pop_front() {
                lines.push(ql.text);
            }
        }
        lines
    }

    /// Drain ALL remaining content (queue + partial line). Called at end of stream.
    pub fn drain_all(&mut self) -> Vec<String> {
        let mut lines: Vec<String> = self.queue.drain(..)
            .map(|ql| ql.text)
            .collect();
        if !self.collector.is_empty() {
            lines.push(std::mem::take(&mut self.collector));
        }
        self.active = false;
        self.mode = ChunkingMode::Smooth;
        self.last_exit_catchup = None;
        lines
    }

    /// Reset state (e.g., on interrupt).
    pub fn reset(&mut self) {
        self.collector.clear();
        self.queue.clear();
        self.active = false;
        self.mode = ChunkingMode::Smooth;
        self.last_exit_catchup = None;
    }
}
