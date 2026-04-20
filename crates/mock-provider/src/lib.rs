//! Deterministic mock Anthropic-compatible service for parity testing.
//!
//! Speaks the Anthropic `/v1/messages` protocol with scripted responses
//! driven by scenario detection from message content.
//!
//! # Usage
//!
//! ```no_run
//! # async fn example() {
//! let service = decipher_mock_provider::MockProviderService::spawn().await.unwrap();
//! println!("Mock running at {}", service.base_url());
//! // ... run tests against service.base_url() ...
//! service.shutdown().await;
//! # }
//! ```

pub mod scenarios;
pub mod sse;
pub mod types;

use scenarios::detect_scenario;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex};
use types::MessageRequest;

/// A captured HTTP request for post-test inspection.
#[derive(Debug, Clone)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub scenario: String,
    pub stream: bool,
    pub raw_body: String,
}

/// Deterministic mock Anthropic-compatible HTTP service.
pub struct MockProviderService {
    base_url: String,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: Option<tokio::task::JoinHandle<()>>,
}

impl MockProviderService {
    /// Spawn the mock service on any available port.
    pub async fn spawn() -> io::Result<Self> {
        Self::spawn_on("127.0.0.1:0").await
    }

    /// Spawn the mock service on a specific address.
    pub async fn spawn_on(addr: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let base_url = format!("http://{local_addr}");
        let requests: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let reqs = requests.clone();
        let handle = tokio::spawn(async move {
            run_server(listener, reqs, shutdown_rx).await;
        });

        Ok(Self {
            base_url,
            requests,
            shutdown_tx: Some(shutdown_tx),
            join_handle: Some(handle),
        })
    }

    /// Get the base URL (e.g., `http://127.0.0.1:12345`).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Retrieve all captured requests (for test assertions).
    pub async fn captured_requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().await.clone()
    }

    /// Shut down the service.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.join_handle.take() {
            let _ = h.await;
        }
    }
}

async fn run_server(
    listener: TcpListener,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _)) => {
                        let reqs = requests.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, reqs).await {
                                eprintln!("[mock-provider] connection error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("[mock-provider] accept error: {e}");
                    }
                }
            }
            _ = &mut shutdown_rx => {
                break;
            }
        }
    }
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
) -> io::Result<()> {
    // Read the full HTTP request into a buffer.
    // We use a simple approach: read until we have headers + Content-Length body.
    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 4096];

    // Read until we find the header/body boundary.
    let header_end;
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);

        if let Some(pos) = find_header_end(&buf) {
            header_end = pos;
            break;
        }
    }

    // Parse the request line and headers.
    let header_str = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = header_str.lines();
    let request_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    let method = parts.first().copied().unwrap_or("GET").to_string();
    let path = parts.get(1).copied().unwrap_or("/").to_string();

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_lowercase(), value.trim().to_string());
        }
    }

    // Read body based on Content-Length.
    let content_length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let body_start = header_end + 4; // skip \r\n\r\n
    let body_already = buf.len() - body_start;
    let remaining = content_length.saturating_sub(body_already);

    if remaining > 0 {
        let mut body_buf = vec![0u8; remaining];
        stream.read_exact(&mut body_buf).await?;
        buf.extend_from_slice(&body_buf);
    }

    let raw_body = String::from_utf8_lossy(&buf[body_start..body_start + content_length]).to_string();

    // Parse the request body.
    let message_request: MessageRequest = match serde_json::from_str(&raw_body) {
        Ok(r) => r,
        Err(e) => {
            let error_body = serde_json::json!({
                "type": "error",
                "error": { "type": "invalid_request_error", "message": format!("JSON parse error: {e}") }
            });
            let resp = format_http_response(400, "application/json", &error_body.to_string(), &[]);
            stream.write_all(resp.as_bytes()).await?;
            return Ok(());
        }
    };

    // Detect scenario and capture request.
    let scenario = detect_scenario(&message_request);
    let scenario_name = scenario.map(|s| s.name()).unwrap_or("unknown");

    {
        let mut reqs = requests.lock().await;
        reqs.push(CapturedRequest {
            method: method.clone(),
            path: path.clone(),
            headers: headers.clone(),
            scenario: scenario_name.to_string(),
            stream: message_request.stream,
            raw_body: raw_body.clone(),
        });
    }

    // Build response.
    let Some(scenario) = scenario else {
        let fallback = types::MessageResponse::text("msg_fallback", "No scenario detected.");
        let body = serde_json::to_string(&fallback).unwrap();
        let resp = format_http_response(200, "application/json", &body, &[]);
        stream.write_all(resp.as_bytes()).await?;
        return Ok(());
    };

    if message_request.stream {
        let body = scenarios::build_stream(&message_request, scenario);
        let extra_headers = &[
            ("x-request-id", format!("req_{scenario_name}")),
        ];
        let resp = format_http_response(200, "text/event-stream", &body, extra_headers);
        stream.write_all(resp.as_bytes()).await?;
    } else {
        let response = scenarios::build_response(&message_request, scenario);
        let body = serde_json::to_string(&response).unwrap();
        let extra_headers = &[
            ("request-id", format!("req_{scenario_name}")),
        ];
        let resp = format_http_response(200, "application/json", &body, extra_headers);
        stream.write_all(resp.as_bytes()).await?;
    }

    Ok(())
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn format_http_response(
    status: u16,
    content_type: &str,
    body: &str,
    extra_headers: &[(&str, String)],
) -> String {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Unknown",
    };
    let mut resp = format!(
        "HTTP/1.1 {status} {status_text}\r\ncontent-type: {content_type}\r\n"
    );
    for (key, value) in extra_headers {
        resp.push_str(&format!("{key}: {value}\r\n"));
    }
    resp.push_str(&format!("content-length: {}\r\n", body.len()));
    resp.push_str("connection: close\r\n\r\n");
    resp.push_str(body);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn service_spawns_and_captures_requests() {
        let service = MockProviderService::spawn().await.unwrap();
        let url = format!("{}/v1/messages", service.base_url());

        // Send a streaming_text scenario request.
        let body = serde_json::json!({
            "model": "claude-sonnet-4-5-20250514",
            "max_tokens": 1024,
            "stream": false,
            "messages": [
                { "role": "user", "content": "PARITY_SCENARIO:streaming_text hello" }
            ]
        });

        let client = reqwest::Client::new();
        let resp = client.post(&url).json(&body).send().await.unwrap();
        assert_eq!(resp.status(), 200);

        let json: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(json["type"], "message");
        assert_eq!(json["role"], "assistant");

        // Check captured request.
        let captured = service.captured_requests().await;
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].scenario, "streaming_text");
        assert_eq!(captured[0].method, "POST");

        service.shutdown().await;
    }

    #[tokio::test]
    async fn streaming_response_returns_sse() {
        let service = MockProviderService::spawn().await.unwrap();
        let url = format!("{}/v1/messages", service.base_url());

        let body = serde_json::json!({
            "model": "claude-sonnet-4-5-20250514",
            "max_tokens": 1024,
            "stream": true,
            "messages": [
                { "role": "user", "content": "PARITY_SCENARIO:streaming_text hello" }
            ]
        });

        let client = reqwest::Client::new();
        let resp = client.post(&url).json(&body).send().await.unwrap();
        assert_eq!(resp.status(), 200);

        let text = resp.text().await.unwrap();
        assert!(text.contains("event: message_start"));
        assert!(text.contains("event: content_block_delta"));
        assert!(text.contains("text_delta"));
        assert!(text.contains("event: message_stop"));

        service.shutdown().await;
    }

    #[tokio::test]
    async fn tool_call_roundtrip_two_turns() {
        let service = MockProviderService::spawn().await.unwrap();
        let url = format!("{}/v1/messages", service.base_url());
        let client = reqwest::Client::new();

        // Turn 1: initial request → tool_use
        let body1 = serde_json::json!({
            "model": "claude-sonnet-4-5-20250514",
            "max_tokens": 1024,
            "stream": false,
            "messages": [
                { "role": "user", "content": "PARITY_SCENARIO:tool_call_roundtrip read fixture.txt" }
            ]
        });
        let resp1: serde_json::Value = client.post(&url).json(&body1).send().await.unwrap().json().await.unwrap();
        assert_eq!(resp1["stop_reason"], "tool_use");
        assert_eq!(resp1["content"][0]["type"], "tool_use");
        assert_eq!(resp1["content"][0]["name"], "read_file");

        // Turn 2: feed tool result → final text
        let body2 = serde_json::json!({
            "model": "claude-sonnet-4-5-20250514",
            "max_tokens": 1024,
            "stream": false,
            "messages": [
                { "role": "user", "content": "PARITY_SCENARIO:tool_call_roundtrip" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "toolu_read_fixture", "name": "read_file", "input": {"path": "fixture.txt"} }
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_read_fixture", "content": "alpha parity line", "is_error": false }
                ]}
            ]
        });
        let resp2: serde_json::Value = client.post(&url).json(&body2).send().await.unwrap().json().await.unwrap();
        assert_eq!(resp2["stop_reason"], "end_turn");
        let text = resp2["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("tool_call_roundtrip complete"));
        assert!(text.contains("alpha parity line"));

        let captured = service.captured_requests().await;
        assert_eq!(captured.len(), 2);

        service.shutdown().await;
    }

    #[tokio::test]
    async fn multi_tool_turn_two_tools() {
        let service = MockProviderService::spawn().await.unwrap();
        let url = format!("{}/v1/messages", service.base_url());
        let client = reqwest::Client::new();

        // Turn 1: initial → two tool_use blocks
        let body1 = serde_json::json!({
            "model": "test",
            "max_tokens": 1024,
            "stream": false,
            "messages": [
                { "role": "user", "content": "PARITY_SCENARIO:multi_tool_turn" }
            ]
        });
        let resp1: serde_json::Value = client.post(&url).json(&body1).send().await.unwrap().json().await.unwrap();
        assert_eq!(resp1["stop_reason"], "tool_use");
        assert_eq!(resp1["content"].as_array().unwrap().len(), 2);
        assert_eq!(resp1["content"][0]["name"], "read_file");
        assert_eq!(resp1["content"][1]["name"], "read_file");

        // Turn 2: feed both tool results → final text
        let body2 = serde_json::json!({
            "model": "test",
            "max_tokens": 1024,
            "stream": false,
            "messages": [
                { "role": "user", "content": "PARITY_SCENARIO:multi_tool_turn" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "toolu_read_a", "name": "read_file", "input": {"path":"a.txt"} },
                    { "type": "tool_use", "id": "toolu_read_b", "name": "read_file", "input": {"path":"b.txt"} }
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_read_a", "content": "content_a", "is_error": false },
                    { "type": "tool_result", "tool_use_id": "toolu_read_b", "content": "content_b", "is_error": false }
                ]}
            ]
        });
        let resp2: serde_json::Value = client.post(&url).json(&body2).send().await.unwrap().json().await.unwrap();
        assert_eq!(resp2["stop_reason"], "end_turn");
        let text = resp2["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("multi_tool_turn complete"));
        assert!(text.contains("content_a"));
        assert!(text.contains("content_b"));

        service.shutdown().await;
    }

    #[tokio::test]
    async fn token_usage_reporting_values() {
        let service = MockProviderService::spawn().await.unwrap();
        let url = format!("{}/v1/messages", service.base_url());
        let client = reqwest::Client::new();

        let body = serde_json::json!({
            "model": "test",
            "max_tokens": 1024,
            "stream": false,
            "messages": [
                { "role": "user", "content": "PARITY_SCENARIO:token_usage_reporting" }
            ]
        });
        let resp: serde_json::Value = client.post(&url).json(&body).send().await.unwrap().json().await.unwrap();
        assert_eq!(resp["usage"]["input_tokens"], 142);
        assert_eq!(resp["usage"]["output_tokens"], 37);
        assert_eq!(resp["usage"]["cache_creation_input_tokens"], 10);
        assert_eq!(resp["usage"]["cache_read_input_tokens"], 5);

        service.shutdown().await;
    }

    #[tokio::test]
    async fn streaming_tool_call_has_input_json_delta() {
        let service = MockProviderService::spawn().await.unwrap();
        let url = format!("{}/v1/messages", service.base_url());
        let client = reqwest::Client::new();

        let body = serde_json::json!({
            "model": "test",
            "max_tokens": 1024,
            "stream": true,
            "messages": [
                { "role": "user", "content": "PARITY_SCENARIO:streaming_tool_call" }
            ]
        });
        let resp = client.post(&url).json(&body).send().await.unwrap();
        let text = resp.text().await.unwrap();

        assert!(text.contains("event: message_start"));
        assert!(text.contains("tool_use"));
        assert!(text.contains("input_json_delta"));
        assert!(text.contains("grep_search"));
        assert!(text.contains("event: message_stop"));

        service.shutdown().await;
    }
}
