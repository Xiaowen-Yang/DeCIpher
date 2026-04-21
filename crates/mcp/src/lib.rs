//! Model Context Protocol (MCP) stdio transport client.
//!
//! Implements MCP spec v2024-11-05 for connecting to external tool servers
//! over stdio (JSON-RPC 2.0).
//!
//! # Usage
//! ```no_run
//! use decipher_mcp::{McpConfig, McpClient};
//!
//! async fn example() {
//!     let cfg = McpConfig::load(std::path::Path::new("~/.decipher"));
//!     for server_cfg in cfg.servers {
//!         if let Ok(mut client) = McpClient::connect(&server_cfg).await {
//!             let tools = client.list_tools().await.unwrap_or_default();
//!             println!("Connected to {}: {} tools", server_cfg.name, tools.len());
//!         }
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};

// ── Error type ─────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("RPC error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("Process spawn failed: {0}")]
    Spawn(String),
}

// ── Config types ───────────────────────────────────────────────────────────────

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Top-level MCP configuration loaded from `~/.decipher/mcp.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

impl McpConfig {
    /// Load MCP config from `~/.decipher/mcp.json`.
    /// Returns empty config if the file is missing or malformed.
    pub fn load(decipher_home: &Path) -> Self {
        let path = decipher_home.join("mcp.json");
        let Ok(data) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&data).unwrap_or_default()
    }
}

// ── Tool types ─────────────────────────────────────────────────────────────────

/// A tool exposed by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema", default)]
    pub input_schema: serde_json::Value,
    /// The name of the server that provides this tool.
    #[serde(skip)]
    pub server_name: String,
}

// ── JSON-RPC types ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    params: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    id: Option<serde_json::Value>,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

// ── MCP client ─────────────────────────────────────────────────────────────────

/// An active connection to an MCP server process.
pub struct McpClient {
    pub server_name: String,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    request_id: AtomicU64,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("server_name", &self.server_name)
            .finish_non_exhaustive()
    }
}

impl McpClient {
    /// Spawn the MCP server process and perform the initialize handshake.
    pub async fn connect(config: &McpServerConfig) -> Result<Self, McpError> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| McpError::Spawn(e.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Spawn("no stdin".into()))?;
        let stdout_raw = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Spawn("no stdout".into()))?;
        let stdout = BufReader::new(stdout_raw);

        let mut client = Self {
            server_name: config.name.clone(),
            stdin,
            stdout,
            request_id: AtomicU64::new(1),
        };

        // Send initialize request.
        let init_params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "decipher",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        let resp = client.rpc("initialize", init_params).await?;
        if resp.get("error").is_some() {
            return Err(McpError::Protocol("initialize failed".into()));
        }

        // Send initialized notification (no response expected).
        client
            .send_notification("notifications/initialized", serde_json::json!({}))
            .await?;

        Ok(client)
    }

    /// List all tools exposed by this server.
    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>, McpError> {
        let resp = self.rpc("tools/list", serde_json::json!({})).await?;
        let tools_val = resp
            .get("tools")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        let mut tools: Vec<McpTool> = serde_json::from_value(tools_val)?;
        for t in &mut tools {
            t.server_name = self.server_name.clone();
        }
        Ok(tools)
    }

    /// Call a tool on this server and return its text result.
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<String, McpError> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });
        let resp = self.rpc("tools/call", params).await?;

        // MCP response: { "content": [{ "type": "text", "text": "..." }] }
        let content = resp
            .get("content")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        let mut parts: Vec<String> = Vec::new();
        for block in &content {
            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    parts.push(text.to_string());
                }
            }
        }

        Ok(parts.join("\n"))
    }

    // ── Internal helpers ───────────────────────────────────────────────────────

    async fn rpc(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, McpError> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        // Read response lines until we find the matching id.
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = self.stdout.read_line(&mut buf).await?;
            if n == 0 {
                return Err(McpError::Protocol("server closed stdout".into()));
            }
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            let resp: JsonRpcResponse = serde_json::from_str(trimmed)?;

            // Check id matches.
            let matches = resp
                .id
                .as_ref()
                .map(|rid| match rid {
                    serde_json::Value::Number(n) => n.as_u64() == Some(id),
                    _ => false,
                })
                .unwrap_or(false);

            if !matches {
                // Notification or different request — skip.
                continue;
            }

            if let Some(err) = resp.error {
                return Err(McpError::Rpc {
                    code: err.code,
                    message: err.message,
                });
            }

            return Ok(resp.result.unwrap_or(serde_json::Value::Null));
        }
    }

    async fn send_notification(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), McpError> {
        // Notifications have no id field.
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        let mut line = serde_json::to_string(&notif)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn mcp_config_load_returns_default_when_missing() {
        let cfg = McpConfig::load(Path::new("/nonexistent"));
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn mcp_config_load_parses_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let mcp_json = r#"{
            "servers": [
                {
                    "name": "filesystem",
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
                    "env": {}
                },
                {
                    "name": "github",
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "env": { "GITHUB_TOKEN": "ghp_test" }
                }
            ]
        }"#;
        fs::write(tmp.path().join("mcp.json"), mcp_json).unwrap();
        let cfg = McpConfig::load(tmp.path());
        assert_eq!(cfg.servers.len(), 2);
        assert_eq!(cfg.servers[0].name, "filesystem");
        assert_eq!(cfg.servers[0].command, "npx");
        assert_eq!(cfg.servers[1].name, "github");
        assert_eq!(
            cfg.servers[1].env.get("GITHUB_TOKEN"),
            Some(&"ghp_test".to_string())
        );
    }

    #[test]
    fn mcp_config_empty_servers_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("mcp.json"), r#"{"servers":[]}"#).unwrap();
        let cfg = McpConfig::load(tmp.path());
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn jsonrpc_request_serializes_correctly() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "tools/list".to_string(),
            params: serde_json::json!({}),
        };
        let s = serde_json::to_string(&req).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 1);
        assert_eq!(v["method"], "tools/list");
    }

    #[test]
    fn mcp_tool_has_server_name_after_list() {
        let mut tool: McpTool = serde_json::from_value(serde_json::json!({
            "name": "read_file",
            "description": "Read a file",
            "inputSchema": {}
        }))
        .unwrap();
        tool.server_name = "filesystem".to_string();
        assert_eq!(tool.server_name, "filesystem");
        assert_eq!(tool.name, "read_file");
    }
}
