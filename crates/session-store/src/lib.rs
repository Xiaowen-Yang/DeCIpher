//! Append-only JSONL session history for DeCIpher.
//!
//! Each session is stored as `~/.decipher/sessions/<thread_id>.jsonl`.
//! Records:
//!   - First line: `session_meta` (thread_id, model, workspace, started_at)
//!   - Middle lines: `event` (ts + ServerMessage payload)
//!   - Last line: `session_end` (ended_at, outcome)
//!
//! A separate index file (`sessions/index.jsonl`) enables fast session listing
//! without reading every session file.

mod error;
mod event;
mod index;
mod store;

pub use error::StoreError;
pub use event::SessionIndexEntry;
pub use index::list_sessions;
pub use store::SessionStore;
