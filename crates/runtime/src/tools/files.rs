//! read_file, write_file, list_files tool handlers.
//!
//! Port source: `agents/executor/tools.js` read_file / write_file handlers.

use super::{resolve_path, ToolContext, ToolOutput};
use serde_json::Value;
use tokio::fs;

const FILE_LIMIT: usize = 6000;

/// read_file — read a file and return its contents.
pub async fn read(args: &Value, ctx: &ToolContext) -> Result<ToolOutput, crate::RuntimeError> {
    let path_str = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => {
            return Ok(ToolOutput::err(
                "path required",
                "[Tool result: read_file]\nError: `path` argument is required",
            ))
        }
    };

    let path = resolve_path(&ctx.workspace, path_str);

    match fs::read_to_string(&path).await {
        Ok(content) => {
            let truncated = content.len() > FILE_LIMIT;
            let preview = if truncated {
                format!(
                    "{}\n... (file truncated: {} chars total)",
                    &content[..FILE_LIMIT],
                    content.len()
                )
            } else {
                content.clone()
            };
            let llm_text = format!(
                "[Tool result: read_file]\nPath: {}\nContent:\n{}",
                path.display(),
                preview
            );
            Ok(ToolOutput::ok(
                format!("{} ({} chars)", path.display(), content.len()),
                llm_text,
            ))
        }
        Err(e) => Ok(ToolOutput::err(
            format!("Error reading {}", path.display()),
            format!(
                "[Tool result: read_file]\nError reading {}: {}",
                path.display(),
                e
            ),
        )),
    }
}

/// write_file — create or overwrite a file.
pub async fn write(args: &Value, ctx: &ToolContext) -> Result<ToolOutput, crate::RuntimeError> {
    let path_str = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => {
            return Ok(ToolOutput::err(
                "path required",
                "[Tool result: write_file]\nError: `path` argument is required",
            ))
        }
    };
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let path = resolve_path(&ctx.workspace, path_str);

    // Read existing content for diff generation.
    let old_content = if path.exists() {
        fs::read_to_string(&path).await.ok()
    } else {
        None
    };

    // Create parent directories as needed.
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent).await {
            return Ok(ToolOutput::err(
                format!("Failed to create directories for {}", path.display()),
                format!(
                    "[Tool result: write_file]\nError creating directories for {}: {}",
                    path.display(),
                    e
                ),
            ));
        }
    }

    match fs::write(&path, &content).await {
        Ok(()) => {
            let verb = if old_content.is_some() {
                "overwritten"
            } else {
                "created new"
            };

            // Generate a compact diff preview.
            let diff_preview = generate_diff_preview(old_content.as_deref(), &content);

            let mut out = ToolOutput::ok(
                format!("Wrote {} ({})", path.display(), verb),
                format!(
                    "[Tool result: write_file]\nWrote: {} ({})",
                    path.display(),
                    verb
                ),
            );
            if !diff_preview.is_empty() {
                out.raw_output = Some(diff_preview);
            }
            Ok(out)
        }
        Err(e) => Ok(ToolOutput::err(
            format!("Error writing {}", path.display()),
            format!(
                "[Tool result: write_file]\nError writing {}: {}",
                path.display(),
                e
            ),
        )),
    }
}

/// list_files — list directory entries.
pub async fn list(args: &Value, ctx: &ToolContext) -> Result<ToolOutput, crate::RuntimeError> {
    let path_str = args
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or(".");

    let path = resolve_path(&ctx.workspace, path_str);

    let mut entries = match fs::read_dir(&path).await {
        Ok(rd) => rd,
        Err(e) => {
            return Ok(ToolOutput::err(
                format!("Error listing {}", path.display()),
                format!(
                    "[Tool result: list_files]\nError listing {}: {}",
                    path.display(),
                    e
                ),
            ))
        }
    };

    let mut names: Vec<String> = Vec::new();
    loop {
        match entries.next_entry().await {
            Ok(Some(entry)) => {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry
                    .file_type()
                    .await
                    .map(|t| t.is_dir())
                    .unwrap_or(false);
                names.push(if is_dir {
                    format!("{}/", name)
                } else {
                    name
                });
            }
            Ok(None) => break,
            Err(e) => {
                names.push(format!("(error reading entry: {e})"));
            }
        }
    }
    names.sort();

    let listing = names.join("\n");
    let llm_text = format!(
        "[Tool result: list_files]\nPath: {}\n{} entries:\n{}",
        path.display(),
        names.len(),
        listing
    );

    Ok(ToolOutput::ok(
        format!("{} entries in {}", names.len(), path.display()),
        llm_text,
    ))
}

/// Generate a compact diff preview between old and new file content.
///
/// Returns lines prefixed with `+` (added) and `-` (removed).
/// For new files, shows the first few lines as `+` additions.
fn generate_diff_preview(old: Option<&str>, new: &str) -> String {
    match old {
        None => {
            // New file — show first lines as additions.
            let lines: Vec<&str> = new.lines().collect();
            let count = lines.len();
            let show = count.min(8);
            let mut out = format!("Added {count} lines\n");
            for line in &lines[..show] {
                out.push_str(&format!("+{line}\n"));
            }
            if count > show {
                out.push_str(&format!("(+{} more lines)", count - show));
            }
            out.trim_end().to_string()
        }
        Some(old_text) => {
            let old_lines: Vec<&str> = old_text.lines().collect();
            let new_lines: Vec<&str> = new.lines().collect();

            // Collect changed lines (simple line-by-line diff).
            let mut added = 0usize;
            let mut removed = 0usize;
            let mut diff_lines: Vec<String> = Vec::new();
            let max_len = old_lines.len().max(new_lines.len());
            for i in 0..max_len {
                let old_l = old_lines.get(i).copied();
                let new_l = new_lines.get(i).copied();
                match (old_l, new_l) {
                    (Some(o), Some(n)) if o != n => {
                        diff_lines.push(format!("-{o}"));
                        diff_lines.push(format!("+{n}"));
                        removed += 1;
                        added += 1;
                    }
                    (Some(o), None) => {
                        diff_lines.push(format!("-{o}"));
                        removed += 1;
                    }
                    (None, Some(n)) => {
                        diff_lines.push(format!("+{n}"));
                        added += 1;
                    }
                    _ => {} // unchanged
                }
            }

            if diff_lines.is_empty() {
                return String::new(); // no changes
            }

            let mut out = format!("Added {added} lines, removed {removed} lines\n");
            let show = diff_lines.len().min(12);
            for line in &diff_lines[..show] {
                out.push_str(line);
                out.push('\n');
            }
            if diff_lines.len() > show {
                out.push_str(&format!("({} more changes)", diff_lines.len() - show));
            }
            out.trim_end().to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmpdir() -> (tempfile::TempDir, ToolContext) {
        let dir = tempfile::TempDir::new().unwrap();
        let ctx = ToolContext {
            workspace: dir.path().to_string_lossy().to_string(),
            on_exec_output: None,
            mcp_clients: None,
            api_key: String::new(),
            model: String::new(),
            base_url: None,
            event_tx: None,
            depth: 0,
            policy_mode: decipher_policy::PolicyMode::Auto,
        };
        (dir, ctx)
    }

    #[tokio::test]
    async fn write_and_read_roundtrip() {
        let (dir, ctx) = tmpdir();
        let path = dir.path().join("test.txt");
        let path_str = path.to_string_lossy().to_string();

        let write_args = serde_json::json!({ "path": path_str, "content": "hello world" });
        let wr = write(&write_args, &ctx).await.unwrap();
        assert!(wr.success);
        assert!(wr.llm_text.contains("created new"));

        let read_args = serde_json::json!({ "path": path_str });
        let rd = read(&read_args, &ctx).await.unwrap();
        assert!(rd.success);
        assert!(rd.llm_text.contains("hello world"));
    }

    #[tokio::test]
    async fn write_overwrites_existing_file() {
        let (dir, ctx) = tmpdir();
        let path = dir.path().join("overwrite.txt");
        let path_str = path.to_string_lossy().to_string();

        let args1 = serde_json::json!({ "path": path_str, "content": "version 1" });
        write(&args1, &ctx).await.unwrap();

        let args2 = serde_json::json!({ "path": path_str, "content": "version 2" });
        let wr2 = write(&args2, &ctx).await.unwrap();
        assert!(wr2.success);
        assert!(wr2.llm_text.contains("overwritten"));
    }

    #[tokio::test]
    async fn read_missing_file_is_error() {
        let (_dir, ctx) = tmpdir();
        let args = serde_json::json!({ "path": "/nonexistent/path/file.txt" });
        let rd = read(&args, &ctx).await.unwrap();
        assert!(!rd.success);
    }

    #[tokio::test]
    async fn list_files_shows_entries() {
        let (dir, ctx) = tmpdir();
        let path_str = dir.path().to_string_lossy().to_string();

        // Create two files.
        std::fs::write(dir.path().join("alpha.txt"), "a").unwrap();
        std::fs::write(dir.path().join("beta.txt"), "b").unwrap();

        let args = serde_json::json!({ "path": path_str });
        let out = list(&args, &ctx).await.unwrap();
        assert!(out.success);
        assert!(out.llm_text.contains("alpha.txt"));
        assert!(out.llm_text.contains("beta.txt"));
    }

    #[tokio::test]
    async fn write_creates_parent_dirs() {
        let (dir, ctx) = tmpdir();
        let nested = dir.path().join("a/b/c/file.txt").to_string_lossy().to_string();
        let args = serde_json::json!({ "path": nested, "content": "nested" });
        let wr = write(&args, &ctx).await.unwrap();
        assert!(wr.success, "failed: {}", wr.llm_text);
    }
}
