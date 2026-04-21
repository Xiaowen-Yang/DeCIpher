//! `SessionStore` — append-only JSONL writer for a single session.
//!
//! A background tokio task owns the file handle and receives pre-serialized
//! lines via an mpsc channel, keeping the write path off the critical event loop.

use std::path::Path;

use chrono::Utc;
use decipher_protocol::ServerMessage;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::error::StoreError;
use crate::event::{EventLine, SessionEnd, SessionIndexEntry, SessionMeta};

/// Append-only JSONL session recorder.
///
/// Drop or call `close()` to flush and release the file handle.
/// `close()` also updates `sessions/index.jsonl`.
pub struct SessionStore {
    thread_id: String,
    started_at: chrono::DateTime<Utc>,
    model: String,
    workspace: String,
    base_dir: std::path::PathBuf,
    /// Send pre-serialized JSONL lines to the background writer.
    tx: mpsc::Sender<String>,
    writer_task: tokio::task::JoinHandle<()>,
}

impl SessionStore {
    /// Create a new session file under `<base_dir>/sessions/<uuid>.jsonl`.
    /// The `sessions/` directory is created if it does not exist.
    pub async fn new(
        base_dir: &Path,
        model: &str,
        workspace: &str,
    ) -> Result<Self, StoreError> {
        let thread_id = Uuid::new_v4().to_string();
        let started_at = Utc::now();

        let sessions_dir = base_dir.join("sessions");
        tokio::fs::create_dir_all(&sessions_dir).await?;

        let file_path = sessions_dir.join(format!("{thread_id}.jsonl"));
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await?;

        // Write the meta header as the first line.
        let meta = SessionMeta {
            record_type: "session_meta".into(),
            thread_id: thread_id.clone(),
            started_at,
            model: model.to_string(),
            workspace: workspace.to_string(),
        };
        let mut line = serde_json::to_string(&meta)?;
        line.push('\n');
        file.write_all(line.as_bytes()).await?;

        let (tx, mut rx) = mpsc::channel::<String>(256);

        let writer_task = tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                let _ = file.write_all(line.as_bytes()).await;
            }
            let _ = file.flush().await;
        });

        Ok(SessionStore {
            thread_id,
            started_at,
            model: model.to_string(),
            workspace: workspace.to_string(),
            base_dir: base_dir.to_path_buf(),
            tx,
            writer_task,
        })
    }

    /// Record a `ServerMessage` as a JSONL event line.
    ///
    /// High-frequency streaming messages (`AgentMessageDelta`,
    /// `ExecOutputDelta`, `Spinner`, `AgentStatus`) are silently dropped
    /// to avoid bloating the session file.
    pub fn record(&self, msg: &ServerMessage) {
        match msg {
            ServerMessage::AgentMessageDelta { .. }
            | ServerMessage::ExecOutputDelta { .. }
            | ServerMessage::Spinner { .. }
            | ServerMessage::AgentStatus { .. } => return,
            _ => {}
        }
        if let Ok(json) = serde_json::to_string(&EventLine {
            record_type: "event",
            ts: Utc::now(),
            msg,
        }) {
            let _ = self.tx.try_send(format!("{json}\n"));
        }
    }

    /// Return the UUID string identifying this session.
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    /// Flush and close the session.
    ///
    /// Appends a `session_end` record, waits for the writer task to finish,
    /// then appends an entry to the session index.
    pub async fn close(self, outcome: Option<String>) {
        let ended_at = Utc::now();
        let end = SessionEnd {
            record_type: "session_end".into(),
            ended_at,
            outcome: outcome.clone(),
        };
        if let Ok(json) = serde_json::to_string(&end) {
            // Use blocking send — channel should have capacity; ignore error.
            let _ = self.tx.send(format!("{json}\n")).await;
        }
        // Drop tx to signal the writer task that no more lines are coming.
        drop(self.tx);
        let _ = self.writer_task.await;

        // Append to the session index so listing works without re-reading every file.
        let entry = SessionIndexEntry {
            thread_id: self.thread_id.clone(),
            started_at: self.started_at,
            ended_at: Some(ended_at),
            model: self.model.clone(),
            workspace: self.workspace.clone(),
            outcome,
        };
        let _ = crate::index::append_index_entry(&self.base_dir, &entry).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decipher_protocol::ServerMessage;
    use tempfile::TempDir;

    #[tokio::test]
    async fn creates_jsonl_with_meta_header() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path(), "claude-sonnet-4-6", "/tmp/workspace")
            .await
            .unwrap();
        let thread_id = store.thread_id().to_string();
        store.close(None).await;

        let path = dir
            .path()
            .join("sessions")
            .join(format!("{thread_id}.jsonl"));
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        let first_line = contents.lines().next().unwrap();
        let meta: serde_json::Value = serde_json::from_str(first_line).unwrap();
        assert_eq!(meta["record_type"], "session_meta");
        assert_eq!(meta["thread_id"], thread_id.as_str());
        assert_eq!(meta["model"], "claude-sonnet-4-6");
        assert_eq!(meta["workspace"], "/tmp/workspace");
    }

    #[tokio::test]
    async fn records_events_and_skips_deltas() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path(), "test-model", "/workspace")
            .await
            .unwrap();
        let thread_id = store.thread_id().to_string();

        store.record(&ServerMessage::AgentMessage {
            text: "hello".into(),
        });
        // These must be silently dropped.
        store.record(&ServerMessage::AgentMessageDelta {
            delta: "skip".into(),
        });
        store.record(&ServerMessage::ExecOutputDelta {
            delta: "also skip".into(),
        });
        store.record(&ServerMessage::Spinner {
            label: "thinking…".into(),
            done: false,
        });

        store.close(Some("PASS".into())).await;

        let path = dir
            .path()
            .join("sessions")
            .join(format!("{thread_id}.jsonl"));
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = contents.lines().collect();

        // meta + 1 event (AgentMessage) + session_end = 3 lines
        assert_eq!(lines.len(), 3, "lines: {lines:?}");

        let event: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(event["record_type"], "event");
        assert_eq!(event["msg"]["type"], "agent_message");
        assert_eq!(event["msg"]["text"], "hello");

        let end: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(end["record_type"], "session_end");
        assert_eq!(end["outcome"], "PASS");
    }

    #[tokio::test]
    async fn index_updated_on_close() {
        let dir = TempDir::new().unwrap();
        let store = SessionStore::new(dir.path(), "model", "/workspace")
            .await
            .unwrap();
        let thread_id = store.thread_id().to_string();
        store.close(Some("FAIL".into())).await;

        let sessions = crate::index::list_sessions(dir.path()).await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].thread_id, thread_id);
        assert_eq!(sessions[0].outcome, Some("FAIL".into()));
        assert!(sessions[0].ended_at.is_some());
    }

    #[tokio::test]
    async fn multiple_sessions_sorted_most_recent_first() {
        let dir = TempDir::new().unwrap();

        let s1 = SessionStore::new(dir.path(), "m", "/w").await.unwrap();
        s1.close(Some("PASS".into())).await;

        // Small sleep so started_at timestamps differ.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        let s2 = SessionStore::new(dir.path(), "m", "/w").await.unwrap();
        let id2 = s2.thread_id().to_string();
        s2.close(Some("FAIL".into())).await;

        let sessions = crate::index::list_sessions(dir.path()).await;
        assert_eq!(sessions.len(), 2);
        // Most recent (s2) should be first.
        assert_eq!(sessions[0].thread_id, id2);
    }
}
