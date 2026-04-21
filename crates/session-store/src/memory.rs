//! Per-project persistent memory for DeCIpher.
//!
//! Memory is stored as `~/.decipher/memory/<project-hash>/memories.jsonl`
//! where `project-hash` is a 16-char hex hash (FNV-1a) of the workspace path.
//!
//! Each line is a JSON object:
//! ```json
//! { "id": "<uuid>", "content": "...", "created_at": "<iso8601>" }
//! ```

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::StoreError;

/// A single memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub created_at: String,
}

/// Per-project memory store.
pub struct MemoryStore {
    file_path: PathBuf,
}

impl MemoryStore {
    /// Open (or create) a memory store for the given workspace.
    ///
    /// Uses a short hex hash of the workspace path as the directory name.
    pub fn new(decipher_home: &Path, workspace: &str) -> Result<Self, StoreError> {
        let hash = path_hash(workspace);
        let dir = decipher_home.join("memory").join(&hash);
        std::fs::create_dir_all(&dir)?;
        let file_path = dir.join("memories.jsonl");
        Ok(Self { file_path })
    }

    /// Add a new memory entry. Returns the generated id.
    pub fn add(&self, content: &str) -> Result<String, StoreError> {
        let id = Uuid::new_v4().to_string();
        let entry = MemoryEntry {
            id: id.clone(),
            content: content.to_string(),
            created_at: Utc::now().to_rfc3339(),
        };
        let line = serde_json::to_string(&entry)?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)?;
        writeln!(file, "{line}")?;
        Ok(id)
    }

    /// List all memory entries in insertion order.
    pub fn list(&self) -> Result<Vec<MemoryEntry>, StoreError> {
        if !self.file_path.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&self.file_path)?;
        let mut entries = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<MemoryEntry>(line) {
                Ok(entry) => entries.push(entry),
                Err(e) => eprintln!("[memory] skipping corrupt line: {e}"),
            }
        }
        Ok(entries)
    }

    /// Clear all memory entries.
    pub fn clear(&self) -> Result<(), StoreError> {
        if self.file_path.exists() {
            std::fs::write(&self.file_path, "")?;
        }
        Ok(())
    }

    /// Format all memories as a string suitable for system prompt injection.
    pub fn load_all_for_injection(&self) -> Result<String, StoreError> {
        let entries = self.list()?;
        if entries.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        for entry in &entries {
            out.push_str(&format!("- {}\n", entry.content));
        }
        Ok(out.trim_end().to_string())
    }
}

/// Compute a short hex hash of the workspace path for directory naming.
fn path_hash(workspace: &str) -> String {
    // Simple FNV-1a hash — no crypto dep needed.
    let mut h: u64 = 14695981039346656037;
    for byte in workspace.bytes() {
        h ^= u64(byte);
        h = h.wrapping_mul(1099511628211);
    }
    format!("{h:016x}")
}

#[allow(clippy::cast_lossless)]
fn u64(b: u8) -> u64 {
    b as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_list_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path(), "/workspace/myproject").unwrap();

        let id1 = store.add("Deploy with zero downtime").unwrap();
        let id2 = store.add("Always run tests before merging").unwrap();

        let entries = store.list().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, id1);
        assert_eq!(entries[0].content, "Deploy with zero downtime");
        assert_eq!(entries[1].id, id2);
    }

    #[test]
    fn clear_removes_all_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path(), "/workspace/proj").unwrap();

        store.add("Remember A").unwrap();
        store.add("Remember B").unwrap();
        assert_eq!(store.list().unwrap().len(), 2);

        store.clear().unwrap();
        assert_eq!(store.list().unwrap().len(), 0);
    }

    #[test]
    fn load_all_for_injection_formats_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path(), "/workspace/proj").unwrap();

        store.add("Use multi-stage Docker builds").unwrap();
        store.add("Keep secrets in environment variables").unwrap();

        let injected = store.load_all_for_injection().unwrap();
        assert!(injected.contains("Use multi-stage Docker builds"));
        assert!(injected.contains("Keep secrets in environment variables"));
        assert!(injected.starts_with('-'));
    }

    #[test]
    fn empty_store_returns_empty_injection() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path(), "/workspace/proj").unwrap();
        let injected = store.load_all_for_injection().unwrap();
        assert!(injected.is_empty());
    }

    #[test]
    fn different_workspaces_get_different_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let store1 = MemoryStore::new(tmp.path(), "/workspace/proj1").unwrap();
        let store2 = MemoryStore::new(tmp.path(), "/workspace/proj2").unwrap();

        store1.add("proj1 memory").unwrap();
        store2.add("proj2 memory").unwrap();

        let entries1 = store1.list().unwrap();
        let entries2 = store2.list().unwrap();
        assert_eq!(entries1.len(), 1);
        assert_eq!(entries2.len(), 1);
        assert_ne!(entries1[0].content, entries2[0].content);
    }

    #[test]
    fn list_returns_empty_for_nonexistent_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path(), "/workspace/newproj").unwrap();
        let entries = store.list().unwrap();
        assert!(entries.is_empty());
    }
}
