//! search, grep_search, file_search tool handlers.
//!
//! Uses `rg` (ripgrep) and `find` via subprocess if available,
//! falling back to a simple built-in for basic cases.
//!
//! Port source: `agents/executor/tools.js` search/grep handlers
//! (which shell out to `grep -r` and `find`).

use super::{resolve_path, ToolContext, ToolOutput};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// search — full-text search across files.
pub async fn search(args: &Value, ctx: &ToolContext) -> Result<ToolOutput, crate::RuntimeError> {
    let query = match args.get("query").and_then(Value::as_str) {
        Some(q) => q.to_string(),
        None => {
            return Ok(ToolOutput::err(
                "query required",
                "[Tool result: search]\nError: `query` argument is required",
            ))
        }
    };

    let dir = args
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or(".")
        .to_string();
    let search_dir = resolve_path(&ctx.workspace, &dir);

    // Try rg first, fall back to grep.
    let result = if which_rg() {
        run_cmd(
            &format!(
                "rg --no-heading -n -- {} {}",
                shell_escape(&query),
                search_dir.display()
            ),
            &std::path::PathBuf::from(&ctx.workspace),
            15,
        )
        .await
    } else {
        run_cmd(
            &format!(
                "grep -rn -- {} {}",
                shell_escape(&query),
                search_dir.display()
            ),
            &std::path::PathBuf::from(&ctx.workspace),
            15,
        )
        .await
    };

    let output = truncate_output(&result.output, 4000);
    let success = result.exit_code == 0 || result.exit_code == 1; // grep exits 1 = no match
    let llm_text = format!(
        "[Tool result: search]\nQuery: {query}\nPath: {dir}\n{}",
        if output.is_empty() { "(no matches)" } else { &output }
    );
    let summary = if output.is_empty() {
        "no matches".to_string()
    } else {
        format!("{} result lines", output.lines().count())
    };

    Ok(ToolOutput {
        success,
        summary,
        llm_text,
        exit_code: Some(result.exit_code),
        raw_output: Some(result.output),
        parsed_output: None,
    })
}

/// grep_search — regex search across files.
pub async fn grep(args: &Value, ctx: &ToolContext) -> Result<ToolOutput, crate::RuntimeError> {
    let pattern = match args.get("pattern").and_then(Value::as_str) {
        Some(p) => p.to_string(),
        None => {
            return Ok(ToolOutput::err(
                "pattern required",
                "[Tool result: grep_search]\nError: `pattern` argument is required",
            ))
        }
    };

    let dir = args
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or(".")
        .to_string();
    let include = args.get("include").and_then(Value::as_str);
    let search_dir = resolve_path(&ctx.workspace, &dir);

    let cmd = if which_rg() {
        let glob_flag = include
            .map(|g| format!(" --glob={}", shell_escape(g)))
            .unwrap_or_default();
        format!(
            "rg --no-heading -n{} -- {} {}",
            glob_flag,
            shell_escape(&pattern),
            search_dir.display()
        )
    } else {
        let include_flag = include
            .map(|g| format!(" --include={}", shell_escape(g)))
            .unwrap_or_default();
        format!(
            "grep -rn{} -E -- {} {}",
            include_flag,
            shell_escape(&pattern),
            search_dir.display()
        )
    };

    let result = run_cmd(&cmd, &std::path::PathBuf::from(&ctx.workspace), 15).await;
    let output = truncate_output(&result.output, 4000);
    let success = result.exit_code == 0 || result.exit_code == 1;
    let llm_text = format!(
        "[Tool result: grep_search]\nPattern: {pattern}\nPath: {dir}\n{}",
        if output.is_empty() { "(no matches)" } else { &output }
    );
    let summary = if output.is_empty() {
        "no matches".to_string()
    } else {
        format!("{} result lines", output.lines().count())
    };

    Ok(ToolOutput {
        success,
        summary,
        llm_text,
        exit_code: Some(result.exit_code),
        raw_output: Some(result.output),
        parsed_output: None,
    })
}

/// file_search — find files by name pattern.
pub async fn file_search(
    args: &Value,
    ctx: &ToolContext,
) -> Result<ToolOutput, crate::RuntimeError> {
    let pattern = match args.get("pattern").and_then(Value::as_str) {
        Some(p) => p.to_string(),
        None => {
            return Ok(ToolOutput::err(
                "pattern required",
                "[Tool result: file_search]\nError: `pattern` argument is required",
            ))
        }
    };

    let dir = args
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or(".")
        .to_string();
    let search_dir = resolve_path(&ctx.workspace, &dir);

    // Use `find` or `fd` if available.
    let cmd = if which_fd() {
        format!("fd --glob {} {}", shell_escape(&pattern), search_dir.display())
    } else {
        format!(
            "find {} -name {}",
            search_dir.display(),
            shell_escape(&pattern)
        )
    };

    let result = run_cmd(&cmd, &std::path::PathBuf::from(&ctx.workspace), 15).await;
    let output = truncate_output(&result.output, 4000);
    let success = result.exit_code == 0;
    let llm_text = format!(
        "[Tool result: file_search]\nPattern: {pattern}\nPath: {dir}\n{}",
        if output.is_empty() { "(no matches)" } else { &output }
    );
    let summary = if output.is_empty() {
        "no files found".to_string()
    } else {
        format!("{} files found", output.lines().count())
    };

    Ok(ToolOutput {
        success,
        summary,
        llm_text,
        exit_code: Some(result.exit_code),
        raw_output: Some(result.output),
        parsed_output: None,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

struct ShellResult {
    exit_code: i32,
    output: String,
}

async fn run_cmd(
    cmd: &str,
    workdir: &std::path::Path,
    timeout_secs: u64,
) -> ShellResult {
    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(workdir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return ShellResult {
                exit_code: 1,
                output: format!("spawn error: {e}"),
            }
        }
    };

    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_end(&mut stdout_buf).await;
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_end(&mut stderr_buf).await;
    }

    let mut output = String::from_utf8_lossy(&stdout_buf).to_string();
    output.push_str(&String::from_utf8_lossy(&stderr_buf));

    let exit_code = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait(),
    )
    .await
    {
        Ok(Ok(status)) => status.code().unwrap_or(-1),
        Ok(Err(_)) => 1,
        Err(_) => {
            let _ = child.kill().await;
            124
        }
    };

    ShellResult {
        exit_code,
        output: output.trim_end().to_string(),
    }
}

fn which_rg() -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v rg")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn which_fd() -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v fd")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Wrap a string in single quotes, escaping any existing single quotes.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn truncate_output(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}\n... (truncated)", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        ToolContext {
            workspace: std::env::temp_dir().to_string_lossy().to_string(),
            on_exec_output: None,
            mcp_clients: None,
            api_key: String::new(),
            model: String::new(),
            base_url: None,
            event_tx: None,
            depth: 0,
        }
    }

    #[test]
    fn shell_escape_basic() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_with_single_quote() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[tokio::test]
    async fn file_search_finds_txt_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("alpha.txt"), "a").unwrap();
        std::fs::write(dir.path().join("beta.txt"), "b").unwrap();
        std::fs::write(dir.path().join("gamma.rs"), "c").unwrap();

        let ctx = ToolContext {
            workspace: dir.path().to_string_lossy().to_string(),
            on_exec_output: None,
            mcp_clients: None,
            api_key: String::new(),
            model: String::new(),
            base_url: None,
            event_tx: None,
            depth: 0,
        };
        let args = serde_json::json!({
            "pattern": "*.txt",
            "path": dir.path().to_string_lossy().as_ref()
        });
        let out = file_search(&args, &ctx).await.unwrap();
        assert!(out.success);
        assert!(out.raw_output.as_deref().unwrap_or("").contains("alpha.txt"));
        assert!(out.raw_output.as_deref().unwrap_or("").contains("beta.txt"));
    }

    #[tokio::test]
    async fn grep_search_finds_pattern() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("code.rs"), "fn hello_world() {}").unwrap();

        let ctx = ToolContext {
            workspace: dir.path().to_string_lossy().to_string(),
            on_exec_output: None,
            mcp_clients: None,
            api_key: String::new(),
            model: String::new(),
            base_url: None,
            event_tx: None,
            depth: 0,
        };
        let args = serde_json::json!({
            "pattern": "hello_world",
            "path": dir.path().to_string_lossy().as_ref()
        });
        let out = grep(&args, &ctx).await.unwrap();
        assert!(out.raw_output.as_deref().unwrap_or("").contains("hello_world"));
    }
}
