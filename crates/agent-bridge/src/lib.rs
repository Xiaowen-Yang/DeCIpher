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
                io::Error::new(io::ErrorKind::Other, format!("Failed to start agent: {e}"))
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

        // Stderr reader: forward as error messages
        tokio::spawn(async move {
            let mut lines = BufReader::new(child_stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = agent_tx.send(ServerMessage::Error { message: line }).await;
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
