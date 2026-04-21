//! Tool schema definitions, classification, and registry for DeCIpher agents.
//!
//! # Overview
//!
//! This crate is the **authoritative source** for:
//!
//! - Every tool the LLM agent can invoke (`ToolName` enum)
//! - JSON Schema definitions for each tool's arguments (`ToolSpec`)
//! - Risk-class classification (`ToolClass`) for policy integration
//!
//! # Migration note (R1 → R2)
//!
//! This replaces `agents/executor/tool-schemas.js`. In R2 (`crates/runtime`),
//! `all_tool_specs()` will be used to build the provider `tools` array for each
//! API request, replacing `buildToolsForProvider()` in JS.
//!
//! `crates/tui/src/cell.rs` currently has its own `is_read_only_tool()` fn.
//! Once the TUI→tools dependency is approved in R2, that fn will be replaced
//! by `classify::is_read_only_by_name()`.

pub mod classify;
pub mod spec;

pub use classify::{is_destructive, is_exec, is_read_only, is_read_only_by_name, is_write, tool_class};
pub use spec::{all_tool_specs, ToolName, ToolSpec};
