//! JSONL record types for the session file and index.

use chrono::{DateTime, Utc};
use decipher_protocol::ServerMessage;
use serde::{Deserialize, Serialize};

/// First line of every session file.
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionMeta {
    pub record_type: String, // "session_meta"
    pub thread_id: String,
    pub started_at: DateTime<Utc>,
    pub model: String,
    pub workspace: String,
}

/// A timestamped ServerMessage event.  Written as a JSONL line.
#[derive(Serialize)]
pub(crate) struct EventLine<'a> {
    pub record_type: &'static str, // "event"
    pub ts: DateTime<Utc>,
    pub msg: &'a ServerMessage,
}

/// Last line of every session file.
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionEnd {
    pub record_type: String, // "session_end"
    pub ended_at: DateTime<Utc>,
    pub outcome: Option<String>,
}

/// One entry in `sessions/index.jsonl`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionIndexEntry {
    pub thread_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub model: String,
    pub workspace: String,
    pub outcome: Option<String>,
}
