//! Append-only JSONL session history for DeCIpher.
//!
//! Each session is stored as `~/.decipher/sessions/<thread_id>.jsonl`.
//! Records:
//!   - First line: `session_meta` (thread_id, model, workspace, mission_goal, started_at)
//!   - Middle lines: `event` (ts + ServerMessage payload)
//!   - Last line: `session_end` (ended_at, outcome)
//!
//! A separate index file (`sessions/index.jsonl`) enables fast session listing
//! without reading every session file.

mod error;
mod event;
mod index;
pub mod load;
pub mod memory;
mod store;

pub use error::StoreError;
pub use event::{SessionIndexEntry, SessionMeta};
pub use index::list_sessions;
pub use load::load_session;
pub use memory::{MemoryEntry, MemoryStore};
pub use store::SessionStore;
