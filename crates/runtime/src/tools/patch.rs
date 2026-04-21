//! apply_patch tool handler — unified diff application.
//!
//! Port source: `agents/executor/tools.js` apply_patch handler.
//! This is a self-contained Rust implementation; no external patch binary required.

use super::{resolve_path, ToolContext, ToolOutput};
use serde_json::Value;
use tokio::fs;

/// apply_patch — apply a unified diff to a file in the workspace.
pub async fn apply(args: &Value, ctx: &ToolContext) -> Result<ToolOutput, crate::RuntimeError> {
    let patch = match args.get("patch").and_then(Value::as_str) {
        Some(p) => p.to_string(),
        None => {
            return Ok(ToolOutput::err(
                "patch required",
                "[Tool result: apply_patch]\nError: `patch` argument is required",
            ))
        }
    };

    if patch.trim().is_empty() {
        return Ok(ToolOutput::err(
            "empty patch",
            "[Tool result: apply_patch]\nError: Empty patch",
        ));
    }

    // Determine target file from explicit arg or patch header.
    let target_path = if let Some(explicit) = args.get("target_file").and_then(Value::as_str) {
        resolve_path(&ctx.workspace, explicit)
    } else {
        // Parse `+++ b/path` or `+++ path` from the patch header.
        let found = patch.lines().find_map(|line| {
            if let Some(rest) = line.strip_prefix("+++ b/") {
                Some(rest.trim().to_string())
            } else if let Some(rest) = line.strip_prefix("+++ ") {
                let s = rest.trim();
                if s != "/dev/null" {
                    Some(s.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        });
        match found {
            Some(rel) => resolve_path(&ctx.workspace, &rel),
            None => {
                return Ok(ToolOutput::err(
                    "cannot determine target file",
                    "[Tool result: apply_patch]\nError: Cannot determine target file from patch header. Add a `target_file` argument or use proper `+++ b/path` headers.",
                ))
            }
        }
    };

    // Read the current file content (may not exist for new files).
    let original = match fs::read_to_string(&target_path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Ok(ToolOutput::err(
                format!("Error reading {}", target_path.display()),
                format!(
                    "[Tool result: apply_patch]\nError reading {}: {}",
                    target_path.display(),
                    e
                ),
            ))
        }
    };

    // Apply the patch.
    match apply_unified_diff(&original, &patch) {
        Ok(patched) => {
            // Create parent directories if needed.
            if let Some(parent) = target_path.parent() {
                if let Err(e) = fs::create_dir_all(parent).await {
                    return Ok(ToolOutput::err(
                        "mkdir failed",
                        format!(
                            "[Tool result: apply_patch]\nError creating directories for {}: {}",
                            target_path.display(),
                            e
                        ),
                    ));
                }
            }
            if let Err(e) = fs::write(&target_path, &patched).await {
                return Ok(ToolOutput::err(
                    format!("Write failed: {}", target_path.display()),
                    format!(
                        "[Tool result: apply_patch]\nPatch failed (write error): {}",
                        e
                    ),
                ));
            }
            Ok(ToolOutput::ok(
                format!("Patched {}", target_path.display()),
                format!(
                    "[Tool result: apply_patch]\nPatch applied to: {}",
                    target_path.display()
                ),
            ))
        }
        Err(e) => Ok(ToolOutput::err(
            format!("Patch failed: {e}"),
            format!("[Tool result: apply_patch]\nPatch failed: {e}"),
        )),
    }
}

// ── Unified diff applier ──────────────────────────────────────────────────────

/// Apply a unified diff to `original` text.  Returns the patched text.
///
/// Supports standard `@@ -a,b +c,d @@` hunks.  Context lines (space-prefixed)
/// are verified for correctness; added lines (`+`) are inserted; removed lines
/// (`-`) are deleted.
pub fn apply_unified_diff(original: &str, patch: &str) -> Result<String, String> {
    // Split original into lines preserving endings.
    let orig_lines: Vec<&str> = original.lines().collect();

    // Parse hunks from the patch.
    let hunks = parse_hunks(patch)?;

    if hunks.is_empty() {
        return Err("No hunks found in patch".to_string());
    }

    // Apply hunks in reverse order (by original line) so earlier hunks don't
    // shift line numbers for later ones.
    let mut result: Vec<String> = orig_lines.iter().map(|&s| s.to_string()).collect();

    // We need to apply in original order but track the offset.
    let mut offset: i64 = 0;

    for hunk in &hunks {
        let orig_start = hunk.orig_start as i64 - 1; // 0-indexed
        let orig_len = hunk.orig_len as i64;

        // The adjusted start in the result vector.
        let result_start = (orig_start + offset) as usize;
        let result_end = result_start + orig_len as usize;

        if result_end > result.len() {
            return Err(format!(
                "Hunk extends past end of file (hunk expects lines {}-{}, file has {} lines)",
                result_start + 1,
                result_end,
                result.len()
            ));
        }

        // Verify context lines and collect the new content.
        let mut new_lines: Vec<String> = Vec::new();
        let mut orig_idx = result_start;

        for line in &hunk.lines {
            match line.kind {
                LineKind::Context => {
                    // Verify the context matches.
                    if orig_idx >= result.len() {
                        return Err(format!(
                            "Context line {} out of range (file has {} lines)",
                            orig_idx + 1,
                            result.len()
                        ));
                    }
                    let expected = line.content.trim_end();
                    let actual = result[orig_idx].trim_end();
                    if expected != actual {
                        // Fuzzy match: allow trailing whitespace differences.
                        if expected.trim() != actual.trim() {
                            return Err(format!(
                                "Context mismatch at line {}: expected {:?}, got {:?}",
                                orig_idx + 1,
                                expected,
                                actual
                            ));
                        }
                    }
                    new_lines.push(result[orig_idx].clone());
                    orig_idx += 1;
                }
                LineKind::Added => {
                    new_lines.push(line.content.clone());
                }
                LineKind::Removed => {
                    if orig_idx >= result.len() {
                        return Err(format!(
                            "Removed line {} out of range",
                            orig_idx + 1
                        ));
                    }
                    // Just skip — don't add to new_lines.
                    orig_idx += 1;
                }
            }
        }

        // Splice the hunk into the result.
        let added_len = new_lines.len() as i64;
        let removed_len = orig_len;
        result.splice(result_start..result_end, new_lines);

        offset += added_len - removed_len;
    }

    // Re-join with newline.
    let mut out = result.join("\n");
    // Preserve trailing newline if original had one.
    if original.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }

    Ok(out)
}

#[derive(Debug)]
#[allow(dead_code)]
struct Hunk {
    orig_start: u32,
    orig_len: u32,
    new_start: u32,
    new_len: u32,
    lines: Vec<PatchLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug)]
struct PatchLine {
    kind: LineKind,
    content: String,
}

fn parse_hunks(patch: &str) -> Result<Vec<Hunk>, String> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut in_hunk = false;

    for raw_line in patch.lines() {
        if raw_line.starts_with("@@ ") {
            // Flush previous hunk.
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            // Parse @@ -orig_start,orig_len +new_start,new_len @@
            let hunk = parse_hunk_header(raw_line)?;
            current = Some(hunk);
            in_hunk = true;
        } else if in_hunk {
            let hunk = current.as_mut().unwrap();
            if raw_line.starts_with('+') {
                hunk.lines.push(PatchLine {
                    kind: LineKind::Added,
                    content: raw_line[1..].to_string(),
                });
            } else if raw_line.starts_with('-') {
                hunk.lines.push(PatchLine {
                    kind: LineKind::Removed,
                    content: raw_line[1..].to_string(),
                });
            } else if raw_line.starts_with(' ') {
                hunk.lines.push(PatchLine {
                    kind: LineKind::Context,
                    content: raw_line[1..].to_string(),
                });
            } else if raw_line.starts_with('\\') {
                // "\ No newline at end of file" — skip.
            }
            // Lines before the first @@ (file header) are ignored once in_hunk.
        }
    }

    if let Some(h) = current.take() {
        hunks.push(h);
    }

    Ok(hunks)
}

fn parse_hunk_header(line: &str) -> Result<Hunk, String> {
    // Format: @@ -orig_start[,orig_len] +new_start[,new_len] @@ [optional context]
    let inner = line
        .strip_prefix("@@ ")
        .and_then(|s| s.split(" @@").next())
        .ok_or_else(|| format!("Invalid hunk header: {line}"))?;

    let parts: Vec<&str> = inner.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(format!("Invalid hunk header parts: {line}"));
    }

    let (orig_start, orig_len) = parse_range(parts[0].trim_start_matches('-'))?;
    let (new_start, new_len) = parse_range(parts[1].trim_start_matches('+'))?;

    Ok(Hunk {
        orig_start,
        orig_len,
        new_start,
        new_len,
        lines: Vec::new(),
    })
}

fn parse_range(s: &str) -> Result<(u32, u32), String> {
    if let Some((start, len)) = s.split_once(',') {
        let start = start
            .parse::<u32>()
            .map_err(|_| format!("Invalid range start: {s}"))?;
        let len = len
            .parse::<u32>()
            .map_err(|_| format!("Invalid range len: {s}"))?;
        Ok((start, len))
    } else {
        let start = s
            .parse::<u32>()
            .map_err(|_| format!("Invalid range: {s}"))?;
        Ok((start, 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_simple_add_line() {
        let original = "line1\nline2\nline3\n";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,4 @@
 line1
 line2
+new_line
 line3
";
        let result = apply_unified_diff(original, patch).unwrap();
        assert_eq!(result, "line1\nline2\nnew_line\nline3\n");
    }

    #[test]
    fn apply_simple_remove_line() {
        let original = "line1\nline2\nline3\n";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,2 @@
 line1
-line2
 line3
";
        let result = apply_unified_diff(original, patch).unwrap();
        assert_eq!(result, "line1\nline3\n");
    }

    #[test]
    fn apply_replace_line() {
        let original = "foo\nbar\nbaz\n";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
 foo
-bar
+qux
 baz
";
        let result = apply_unified_diff(original, patch).unwrap();
        assert_eq!(result, "foo\nqux\nbaz\n");
    }

    #[test]
    fn apply_context_mismatch_returns_err() {
        let original = "line1\nline2\n";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1,2 +1,2 @@
 wrong_context
-line2
+replaced
";
        assert!(apply_unified_diff(original, patch).is_err());
    }

    #[tokio::test]
    async fn apply_tool_writes_patched_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("hello.txt");
        tokio::fs::write(&file, "hello\nworld\n").await.unwrap();

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
        let patch = "\
--- a/hello.txt
+++ b/hello.txt
@@ -1,2 +1,2 @@
 hello
-world
+rust
";
        let args = serde_json::json!({ "patch": patch });
        let out = apply(&args, &ctx).await.unwrap();
        assert!(out.success, "patch failed: {}", out.llm_text);

        let patched = tokio::fs::read_to_string(&file).await.unwrap();
        assert_eq!(patched, "hello\nrust\n");
    }
}
