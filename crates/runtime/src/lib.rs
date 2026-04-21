//! Rust-native agent execution core for DeCIpher.
//!
//! # Overview
//!
//! This crate is the **R2 migration target**: it moves the agent loop and tool
//! execution from Node.js (`agents/executor/agent-loop.js` + `tools.js`) into
//! Rust, running in-process with the TUI.
//!
//! # Architecture
//!
//! ```text
//! AgentLoop::run(config, event_tx)
//!   └── per turn: Provider::send_message(messages + tools)
//!         └── parse tool_use blocks
//!               └── tools::dispatch(tool, args, workspace)
//!                     └── emit ServerMessage events → event_tx
//! ```
//!
//! The caller supplies a `tokio::sync::mpsc::Sender<ServerMessage>` and an
//! approval callback.  Events flow directly without JSON serialization,
//! replacing the subprocess bridge.
//!
//! # Migration note
//!
//! Until R4 (bridge collapse), the TUI continues to launch Node.js via
//! `crates/agent-bridge`.  This crate provides an **alternative path** that
//! can be activated once the in-process wiring is complete.

pub mod agent_loop;
pub mod compaction;
pub mod hooks;
pub mod instructions;
pub mod output_parser;
pub mod skills;
pub mod tools;
pub mod types;

pub use agent_loop::AgentLoop;
pub use hooks::{HookConfig, fire_session_event};
pub use skills::{Skill, format_skills_section, load_skills};
pub use types::{AgentConfig, RunOutcome, RunResult, RuntimeError};
