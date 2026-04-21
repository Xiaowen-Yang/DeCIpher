//! exec_command and kubectl tool handlers.
//!
//! Port source: `agents/executor/tools.js` exec_command / kubectl_* handlers.

use super::{resolve_path, ToolContext, ToolOutput};
use serde_json::Value;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const EXEC_TIMEOUT_SECS: u64 = 120;
const OUTPUT_LIMIT: usize = 4000;
const OUTPUT_LIMIT_FAILURE: usize = 8000;

/// Run an arbitrary shell command.
pub async fn run(args: &Value, ctx: &ToolContext) -> Result<ToolOutput, crate::RuntimeError> {
    let cmd = args
        .get("cmd")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if cmd.trim().is_empty() {
        return Ok(ToolOutput::err(
            "No command provided",
            "[Tool result: exec_command]\nError: `cmd` argument is required",
        ));
    }

    let workdir = args
        .get("workdir")
        .and_then(Value::as_str)
        .map(|p| resolve_path(&ctx.workspace, p))
        .unwrap_or_else(|| std::path::PathBuf::from(&ctx.workspace));

    let on_output = ctx.on_exec_output.clone();
    let result = exec_shell(&cmd, &workdir, EXEC_TIMEOUT_SECS, on_output).await;

    let limit = if result.exit_code != 0 {
        OUTPUT_LIMIT_FAILURE
    } else {
        OUTPUT_LIMIT
    };
    let output = result.output.clone();
    let truncated = output.len() > limit;
    let preview = if truncated {
        format!(
            "{}\n... (output truncated: {} chars total)",
            &output[..limit],
            output.len()
        )
    } else {
        output.clone()
    };

    let status_tag = if result.exit_code != 0 { " (FAILED)" } else { "" };
    let llm_text = format!(
        "[Tool result: exec_command]\nCommand: {}\nExit code: {}{}\nOutput:\n{}",
        cmd,
        result.exit_code,
        status_tag,
        if preview.is_empty() { "(no output)" } else { &preview }
    );

    let success = result.exit_code == 0;
    let summary = if success {
        format!("exit 0 — {} chars", output.len())
    } else {
        format!("exit {} — {}", result.exit_code, truncate_summary(&output, 80))
    };

    Ok(ToolOutput {
        success,
        summary,
        llm_text,
        exit_code: Some(result.exit_code),
        raw_output: Some(output),
    })
}

/// Run a kubectl subcommand generically (get / describe).
pub async fn kubectl(
    args: &Value,
    ctx: &ToolContext,
    subcommand: &str,
) -> Result<ToolOutput, crate::RuntimeError> {
    let resource = args
        .get("resource")
        .and_then(Value::as_str)
        .unwrap_or("pods");
    let namespace = args.get("namespace").and_then(Value::as_str);
    let output_fmt = args.get("output").and_then(Value::as_str);
    let selector = args.get("selector").and_then(Value::as_str);
    let name = args.get("name").and_then(Value::as_str).unwrap_or("");

    let mut parts: Vec<String> = vec!["kubectl".to_string(), subcommand.to_string()];
    parts.push(resource.to_string());
    if !name.is_empty() {
        parts.push(name.to_string());
    }
    if let Some(ns) = namespace {
        parts.push(format!("-n {ns}"));
    }
    if let Some(fmt) = output_fmt {
        parts.push(format!("-o {fmt}"));
    }
    if let Some(sel) = selector {
        parts.push(format!("-l {sel}"));
    }

    let cmd = parts.join(" ");
    run_kubectl_cmd(&cmd, ctx).await
}

/// Run kubectl logs.
pub async fn kubectl_logs(
    args: &Value,
    ctx: &ToolContext,
) -> Result<ToolOutput, crate::RuntimeError> {
    let pod = match args.get("pod").and_then(Value::as_str) {
        Some(p) => p,
        None => {
            return Ok(ToolOutput::err(
                "pod name required",
                "[Tool result: kubectl_logs]\nError: `pod` argument is required",
            ))
        }
    };
    let namespace = args.get("namespace").and_then(Value::as_str);
    let container = args.get("container").and_then(Value::as_str);
    let previous = args
        .get("previous")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let tail = args.get("tail").and_then(Value::as_u64).unwrap_or(200);

    let mut parts: Vec<String> = vec![
        "kubectl".to_string(),
        "logs".to_string(),
        pod.to_string(),
    ];
    if let Some(ns) = namespace {
        parts.push(format!("-n {ns}"));
    }
    if let Some(c) = container {
        parts.push(format!("-c {c}"));
    }
    if previous {
        parts.push("--previous".to_string());
    }
    parts.push(format!("--tail={tail}"));

    run_kubectl_cmd(&parts.join(" "), ctx).await
}

/// Run kubectl get events.
pub async fn kubectl_events(
    args: &Value,
    ctx: &ToolContext,
) -> Result<ToolOutput, crate::RuntimeError> {
    let namespace = args.get("namespace").and_then(Value::as_str);
    let field_selector = args.get("field_selector").and_then(Value::as_str);

    let ns_flag = namespace
        .map(|ns| format!("-n {ns}"))
        .unwrap_or_else(|| "--all-namespaces".to_string());
    let mut parts: Vec<&str> = vec![
        "kubectl",
        "get",
        "events",
        &ns_flag,
        "--sort-by=.lastTimestamp",
    ];
    let fs_owned;
    if let Some(fs) = field_selector {
        fs_owned = format!("--field-selector={fs}");
        parts.push(&fs_owned);
    }
    let cmd = parts.join(" ");
    run_kubectl_cmd(&cmd, ctx).await
}

// ── Internal helpers ──────────────────────────────────────────────────────────

struct ExecResult {
    exit_code: i32,
    output: String,
}

async fn exec_shell(
    cmd: &str,
    workdir: &std::path::Path,
    timeout_secs: u64,
    on_output: Option<tokio::sync::mpsc::UnboundedSender<String>>,
) -> ExecResult {
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
            return ExecResult {
                exit_code: 1,
                output: format!("Failed to spawn: {e}"),
            }
        }
    };

    // Read stdout and stderr independently, then merge.
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

    if let Some(tx) = &on_output {
        let _ = tx.send(output.clone());
    }

    let exit_code = match tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        child.wait(),
    )
    .await
    {
        Ok(Ok(status)) => status.code().unwrap_or(-1),
        Ok(Err(e)) => {
            output.push_str(&format!("\nProcess wait error: {e}"));
            1
        }
        Err(_) => {
            let _ = child.kill().await;
            output.push_str("\n[Timed out]");
            124
        }
    };

    ExecResult {
        exit_code,
        output: output.trim_end().to_string(),
    }
}

async fn run_kubectl_cmd(cmd: &str, ctx: &ToolContext) -> Result<ToolOutput, crate::RuntimeError> {
    let workdir = std::path::PathBuf::from(&ctx.workspace);
    let result = exec_shell(cmd, &workdir, 30, None).await;
    let success = result.exit_code == 0;
    let llm_text = format!(
        "[Tool result: kubectl]\nCommand: {}\nExit code: {}\nOutput:\n{}",
        cmd,
        result.exit_code,
        if result.output.is_empty() {
            "(no output)"
        } else {
            &result.output
        }
    );
    let summary = if success {
        truncate_summary(&result.output, 80)
    } else {
        format!("exit {}", result.exit_code)
    };
    Ok(ToolOutput {
        success,
        summary,
        llm_text,
        exit_code: Some(result.exit_code),
        raw_output: Some(result.output),
    })
}

fn truncate_summary(s: &str, max: usize) -> String {
    let first_line = s.lines().next().unwrap_or("").trim();
    if first_line.len() <= max {
        first_line.to_string()
    } else {
        format!("{}…", &first_line[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        ToolContext {
            workspace: std::env::temp_dir().to_string_lossy().to_string(),
            on_exec_output: None,
        }
    }

    #[tokio::test]
    async fn exec_echo_succeeds() {
        let args = serde_json::json!({ "cmd": "echo hello_world" });
        let out = run(&args, &ctx()).await.unwrap();
        assert!(out.success);
        assert!(out.raw_output.as_deref().unwrap_or("").contains("hello_world"));
        assert_eq!(out.exit_code, Some(0));
    }

    #[tokio::test]
    async fn exec_nonzero_exit_is_failure() {
        let args = serde_json::json!({ "cmd": "exit 42" });
        let out = run(&args, &ctx()).await.unwrap();
        assert!(!out.success);
        assert_eq!(out.exit_code, Some(42));
        assert!(out.llm_text.contains("FAILED"));
    }

    #[tokio::test]
    async fn exec_empty_cmd_is_error() {
        let args = serde_json::json!({ "cmd": "" });
        let out = run(&args, &ctx()).await.unwrap();
        assert!(!out.success);
    }

    #[tokio::test]
    async fn exec_captures_stderr() {
        let args = serde_json::json!({ "cmd": "echo stderr_line >&2" });
        let out = run(&args, &ctx()).await.unwrap();
        assert!(out.raw_output.as_deref().unwrap_or("").contains("stderr_line"));
    }
}
