//! Session index: fast listing without reading every session file.
//!
//! `sessions/index.jsonl` — one `SessionIndexEntry` per line, append-only.
//! Listing reads all lines and sorts by `started_at` descending.

use std::path::Path;

use tokio::io::AsyncWriteExt;

use crate::error::StoreError;
use crate::event::SessionIndexEntry;

const INDEX_FILE: &str = "sessions/index.jsonl";

/// Append one entry to the index.  Called by `SessionStore::close`.
pub(crate) async fn append_index_entry(
    base_dir: &Path,
    entry: &SessionIndexEntry,
) -> Result<(), StoreError> {
    let index_path = base_dir.join(INDEX_FILE);
    // Parent dir is guaranteed to exist (created by SessionStore::new).
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&index_path)
        .await?;
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    file.write_all(line.as_bytes()).await?;
    Ok(())
}

/// List all recorded sessions, most recent first.
///
/// Returns `Ok(vec![])` when the index does not exist yet (first run).
/// Returns `Err` only for real I/O or parse failures.
pub async fn list_sessions(base_dir: &Path) -> Result<Vec<SessionIndexEntry>, crate::StoreError> {
    let index_path = base_dir.join(INDEX_FILE);
    let contents = match tokio::fs::read_to_string(&index_path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(crate::StoreError::Io(e)),
    };
    let mut entries: Vec<SessionIndexEntry> = contents
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    entries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(entries)
}
