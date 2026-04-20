//! DeCIpher TUI library — app state, rendering, and input handling.

pub mod app;
pub mod render;
pub mod streaming;
pub mod shimmer;
pub mod paste_burst;
pub mod diff_render;
pub mod ansi_escape;
pub mod wrapping;
pub mod pager;
pub mod file_search;
pub mod terminal_detect;

// ── Phase 1: ratatui types (compiling, not yet wired up) ───────────────────
pub mod cell;
pub mod renderable;
pub mod chat;
pub mod markdown_stream;

// ── Phase 2: ratatui viewport rendering ────────────────────────────────────
pub mod bottom_pane;
