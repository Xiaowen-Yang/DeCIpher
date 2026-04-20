//! Node.js agent subprocess management and JSON I/O.
//!
//! Spawns `node bin/decipher --server` and provides typed channels for
//! sending/receiving protocol messages. This is the migration seam:
//! when the Node.js backend is replaced by Rust, this crate swaps
//! subprocess I/O for in-process async channels.

use std::io;
use std::path::PathBuf;

use decipher_protocol::{ClientMessage, ServerMessage};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use std::process::Stdio;

/// Backpressure buffer size for agent→TUI message channel.
/// Prevents OOM on verbose long-running sessions.
const CHANNEL_BUFFER: usize = 1024;

/// Handle to a running agent subprocess.
pub struct AgentBridge {
    pub child: tokio::process::Child,
    pub stdin: tokio::process::ChildStdin,
    pub rx: mpsc::Receiver<ServerMessage>,
}

impl AgentBridge {
    /// Spawn the Node.js agent in server mode and return a bridge handle.
    pub async fn spawn() -> io::Result<Self> {
        let bin_path = find_agent_script();

        let mut child = Command::new("node")
            .arg(&bin_path)
            .arg("--server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                eprintln!("Failed to start agent: {e}\nTried: node {bin_path}");
                io::Error::other(format!("Failed to start agent: {e}"))
            })?;

        let child_stdin = child.stdin.take().expect("child stdin");
        let child_stdout = child.stdout.take().expect("child stdout");
        let child_stderr = child.stderr.take().expect("child stderr");

        let (agent_tx, agent_rx) = mpsc::channel::<ServerMessage>(CHANNEL_BUFFER);

        // Stdout reader: parse JSON messages from agent.
        // ONLY valid JSON protocol messages are forwarded to the TUI.
        // Non-JSON lines (e.g., stray console.log from agent code) are
        // silently dropped — they would corrupt the display.
        let tx1 = agent_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(child_stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match serde_json::from_str::<ServerMessage>(&line) {
                    Ok(msg) => { let _ = tx1.send(msg).await; }
                    Err(_) => {
                        // Drop non-JSON lines. In server mode, all agent
                        // output should go through send() as JSON.
                        // Log to TUI stderr for debugging if needed.
                        #[cfg(debug_assertions)]
                        eprintln!("[bridge] dropped non-JSON: {}", &line[..line.len().min(120)]);
                    }
                }
            }
        });

        // Stderr reader: forward only lines that look like actual errors.
        //
        // Lesson: "Stderr Is Not An Error Channel" — Node.js emits warnings,
        // deprecation notices, spinner output, and display formatting to stderr.
        // Forwarding everything as ServerMessage::Error makes normal operations
        // appear broken in the TUI. Only forward lines containing error patterns.
        tokio::spawn(async move {
            let mut lines = BufReader::new(child_stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if is_error_line(&line) {
                    let _ = agent_tx.send(ServerMessage::Error { message: line }).await;
                }
                // Non-error stderr lines are silently dropped.
                // The source fixes (spinner, result printer) prevent display
                // output from reaching stderr in server-mode.
            }
        });

        Ok(Self {
            child,
            stdin: child_stdin,
            rx: agent_rx,
        })
    }

    /// Send a typed message to the agent.
    pub async fn send(&mut self, msg: &ClientMessage) -> io::Result<()> {
        let json = serde_json::to_string(msg).unwrap();
        self.stdin.write_all(json.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Shut down the agent subprocess.
    pub async fn shutdown(&mut self) {
        let _ = self.child.kill().await;
    }
}

/// Determine if a stderr line looks like an actual error (vs display output).
///
/// Node.js emits deprecation warnings, spinner output, and ANSI-formatted
/// display text to stderr.  Only lines matching error patterns should be
/// forwarded to the TUI as `ServerMessage::Error`.
fn is_error_line(line: &str) -> bool {
    // Strip ANSI escape sequences for pattern matching.
    let stripped = strip_ansi(line);
    let trimmed = stripped.trim();

    // Empty lines or pure whitespace are never errors.
    if trimmed.is_empty() {
        return false;
    }

    // Positive match: known error patterns.
    let lower = trimmed.to_ascii_lowercase();
    lower.contains("error")
        || lower.contains("fatal")
        || lower.contains("unhandled")
        || lower.contains("exception")
        || lower.contains("enoent")
        || lower.contains("eacces")
        || lower.contains("eperm")
        || lower.starts_with("at ")       // stack trace frame
        || lower.starts_with("warn")
        || lower.contains("deprecat")
}

/// Strip ANSI SGR and OSC escape sequences from a string.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    // CSI sequence: consume until letter
                    chars.next();
                    while let Some(&c) = chars.peek() {
                        chars.next();
                        if c.is_ascii_alphabetic() { break; }
                    }
                }
                Some(']') => {
                    // OSC sequence: consume until ST (\x1b\\) or BEL (\x07)
                    chars.next();
                    while let Some(&c) = chars.peek() {
                        if c == '\x07' { chars.next(); break; }
                        if c == '\x1b' {
                            chars.next();
                            if chars.peek() == Some(&'\\') { chars.next(); }
                            break;
                        }
                        chars.next();
                    }
                }
                _ => {}
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Find the Node.js agent script path.
fn find_agent_script() -> String {
    if let Ok(path) = std::env::var("DECIPHER_AGENT_SCRIPT") {
        if PathBuf::from(&path).exists() { return path; }
    }
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(std::path::Path::new("."));
        let c = dir.join("decipher");
        if c.exists() { return c.to_string_lossy().to_string(); }
        let c2 = dir.join("../../bin/decipher");
        if c2.exists() { return c2.canonicalize().unwrap_or(c2).to_string_lossy().to_string(); }
    }
    let c = PathBuf::from("bin/decipher");
    if c.exists() { return c.canonicalize().unwrap_or(c).to_string_lossy().to_string(); }
    "bin/decipher".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_error_line ────────────────────────────────────────────────────

    #[test]
    fn error_line_detects_actual_errors() {
        assert!(is_error_line("Error: connection refused"));
        assert!(is_error_line("TypeError: Cannot read property 'x' of undefined"));
        assert!(is_error_line("FATAL: out of memory"));
        assert!(is_error_line("UnhandledPromiseRejectionWarning: Error"));
        assert!(is_error_line("at Object.<anonymous> (/app/index.js:10:5)"));
        assert!(is_error_line("ENOENT: no such file or directory"));
        assert!(is_error_line("EACCES: permission denied"));
        assert!(is_error_line("Warning: deprecated API"));
    }

    #[test]
    fn error_line_rejects_display_output() {
        // Spinner completion lines
        assert!(!is_error_line("  \u{2713} exec_command (4.9s)"));
        assert!(!is_error_line("  \u{2713} Working in \u{2192} /app"));

        // Result rendering lines
        assert!(!is_error_line("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"));
        assert!(!is_error_line("  [RESULT]"));
        assert!(!is_error_line("  Outcome:     PASS (44.8s)"));
        assert!(!is_error_line("  Turns:       3"));
        assert!(!is_error_line("  Summary:     Successfully cloned the repo"));

        // Plan update lines
        assert!(!is_error_line("  [PLAN]"));
        assert!(!is_error_line("    \u{2713} Read Dockerfile"));

        // Empty / whitespace
        assert!(!is_error_line(""));
        assert!(!is_error_line("   "));

        // Raw protocol JSON (should not be shown as error)
        assert!(!is_error_line(r#"{"type":"exec_output_delta","delta":"Cloning..."}"#));
        assert!(!is_error_line(r#"{"type":"agent_status","phase":"thinking","turn":2}"#));
    }

    #[test]
    fn error_line_handles_ansi_codes() {
        // Error wrapped in ANSI color codes
        assert!(is_error_line("\x1b[31mError: something broke\x1b[0m"));
        // Display output wrapped in ANSI
        assert!(!is_error_line("\x1b[32m\u{2713}\x1b[0m done (2.1s)"));
    }

    #[test]
    fn error_line_handles_osc_sequences() {
        // OSC 8 hyperlink around a URL in an error message
        assert!(is_error_line("Error: failed to fetch \x1b]8;;https://example.com\x07url\x1b]8;;\x07"));
        // OSC 8 in display output
        assert!(!is_error_line("  Summary: cloned \x1b]8;;https://github.com/repo\x07repo\x1b]8;;\x07"));
    }

    // ── strip_ansi ─────────────────��─────────────────────────────────────

    #[test]
    fn strip_ansi_removes_csi() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
        assert_eq!(strip_ansi("\x1b[1;32mbold green\x1b[0m"), "bold green");
    }

    #[test]
    fn strip_ansi_removes_osc() {
        // BEL-terminated OSC
        assert_eq!(strip_ansi("\x1b]8;;https://x.com\x07link\x1b]8;;\x07"), "link");
        // ST-terminated OSC
        assert_eq!(strip_ansi("\x1b]0;title\x1b\\rest"), "rest");
    }

    #[test]
    fn strip_ansi_preserves_plain_text() {
        assert_eq!(strip_ansi("hello world"), "hello world");
        assert_eq!(strip_ansi(""), "");
    }
}
