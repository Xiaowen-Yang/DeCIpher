use crate::types::{Usage, DEFAULT_MODEL};

/// Append one SSE frame: `event: {name}\ndata: {json}\n\n`
pub fn append_sse(buf: &mut String, event: &str, data: &str) {
    buf.push_str("event: ");
    buf.push_str(event);
    buf.push('\n');
    buf.push_str("data: ");
    buf.push_str(data);
    buf.push_str("\n\n");
}

/// Build a complete SSE stream for a plain text response.
pub fn streaming_text(text: &str, usage: &Usage) -> String {
    let chunks = chunk_text(text, 12);
    let mut buf = String::new();

    // message_start
    let msg_start = serde_json::json!({
        "type": "message_start",
        "message": {
            "id": "msg_streaming_text",
            "type": "message",
            "role": "assistant",
            "content": [],
            "model": DEFAULT_MODEL,
            "stop_reason": null,
            "usage": {
                "input_tokens": usage.input_tokens,
                "output_tokens": 0
            }
        }
    });
    append_sse(&mut buf, "message_start", &msg_start.to_string());

    // content_block_start
    let block_start = serde_json::json!({
        "type": "content_block_start",
        "index": 0,
        "content_block": { "type": "text", "text": "" }
    });
    append_sse(&mut buf, "content_block_start", &block_start.to_string());

    // content_block_delta for each chunk
    for chunk in &chunks {
        let delta = serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": chunk }
        });
        append_sse(&mut buf, "content_block_delta", &delta.to_string());
    }

    // content_block_stop
    let block_stop = serde_json::json!({
        "type": "content_block_stop",
        "index": 0
    });
    append_sse(&mut buf, "content_block_stop", &block_stop.to_string());

    // message_delta
    let msg_delta = serde_json::json!({
        "type": "message_delta",
        "delta": { "stop_reason": "end_turn" },
        "usage": { "output_tokens": usage.output_tokens }
    });
    append_sse(&mut buf, "message_delta", &msg_delta.to_string());

    // message_stop
    append_sse(&mut buf, "message_stop", &serde_json::json!({"type":"message_stop"}).to_string());

    buf
}

/// Build a complete SSE stream for a single tool_use response.
pub fn streaming_tool_use(
    tool_id: &str,
    tool_name: &str,
    input_json: &str,
    usage: &Usage,
) -> String {
    let json_chunks = chunk_text(input_json, 20);
    let mut buf = String::new();

    // message_start
    let msg_start = serde_json::json!({
        "type": "message_start",
        "message": {
            "id": "msg_streaming_tool",
            "type": "message",
            "role": "assistant",
            "content": [],
            "model": DEFAULT_MODEL,
            "stop_reason": null,
            "usage": {
                "input_tokens": usage.input_tokens,
                "output_tokens": 0
            }
        }
    });
    append_sse(&mut buf, "message_start", &msg_start.to_string());

    // content_block_start with tool_use
    let block_start = serde_json::json!({
        "type": "content_block_start",
        "index": 0,
        "content_block": {
            "type": "tool_use",
            "id": tool_id,
            "name": tool_name,
            "input": {}
        }
    });
    append_sse(&mut buf, "content_block_start", &block_start.to_string());

    // input_json_delta chunks
    for chunk in &json_chunks {
        let delta = serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "input_json_delta", "partial_json": chunk }
        });
        append_sse(&mut buf, "content_block_delta", &delta.to_string());
    }

    // content_block_stop
    let block_stop = serde_json::json!({
        "type": "content_block_stop",
        "index": 0
    });
    append_sse(&mut buf, "content_block_stop", &block_stop.to_string());

    // message_delta
    let msg_delta = serde_json::json!({
        "type": "message_delta",
        "delta": { "stop_reason": "tool_use" },
        "usage": { "output_tokens": usage.output_tokens }
    });
    append_sse(&mut buf, "message_delta", &msg_delta.to_string());

    // message_stop
    append_sse(&mut buf, "message_stop", &serde_json::json!({"type":"message_stop"}).to_string());

    buf
}

/// Build a complete SSE stream for a multi-tool response (multiple tool_use blocks).
pub fn streaming_multi_tool(
    tools: &[(&str, &str, &str)], // (tool_id, tool_name, input_json)
    usage: &Usage,
) -> String {
    let mut buf = String::new();

    // message_start
    let msg_start = serde_json::json!({
        "type": "message_start",
        "message": {
            "id": "msg_streaming_multi_tool",
            "type": "message",
            "role": "assistant",
            "content": [],
            "model": DEFAULT_MODEL,
            "stop_reason": null,
            "usage": {
                "input_tokens": usage.input_tokens,
                "output_tokens": 0
            }
        }
    });
    append_sse(&mut buf, "message_start", &msg_start.to_string());

    for (idx, (tool_id, tool_name, input_json)) in tools.iter().enumerate() {
        // content_block_start
        let block_start = serde_json::json!({
            "type": "content_block_start",
            "index": idx,
            "content_block": {
                "type": "tool_use",
                "id": tool_id,
                "name": tool_name,
                "input": {}
            }
        });
        append_sse(&mut buf, "content_block_start", &block_start.to_string());

        // input_json_delta (single chunk for multi-tool — keep it simple)
        let delta = serde_json::json!({
            "type": "content_block_delta",
            "index": idx,
            "delta": { "type": "input_json_delta", "partial_json": input_json }
        });
        append_sse(&mut buf, "content_block_delta", &delta.to_string());

        // content_block_stop
        let block_stop = serde_json::json!({
            "type": "content_block_stop",
            "index": idx
        });
        append_sse(&mut buf, "content_block_stop", &block_stop.to_string());
    }

    // message_delta
    let msg_delta = serde_json::json!({
        "type": "message_delta",
        "delta": { "stop_reason": "tool_use" },
        "usage": { "output_tokens": usage.output_tokens }
    });
    append_sse(&mut buf, "message_delta", &msg_delta.to_string());

    // message_stop
    append_sse(&mut buf, "message_stop", &serde_json::json!({"type":"message_stop"}).to_string());

    buf
}

/// Build a complete SSE stream for a final text response (after tool results).
pub fn streaming_final_text(text: &str, usage: &Usage) -> String {
    // Same as streaming_text but with a distinct message id
    let chunks = chunk_text(text, 20);
    let mut buf = String::new();

    let msg_start = serde_json::json!({
        "type": "message_start",
        "message": {
            "id": "msg_final_text",
            "type": "message",
            "role": "assistant",
            "content": [],
            "model": DEFAULT_MODEL,
            "stop_reason": null,
            "usage": {
                "input_tokens": usage.input_tokens,
                "output_tokens": 0
            }
        }
    });
    append_sse(&mut buf, "message_start", &msg_start.to_string());

    let block_start = serde_json::json!({
        "type": "content_block_start",
        "index": 0,
        "content_block": { "type": "text", "text": "" }
    });
    append_sse(&mut buf, "content_block_start", &block_start.to_string());

    for chunk in &chunks {
        let delta = serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": chunk }
        });
        append_sse(&mut buf, "content_block_delta", &delta.to_string());
    }

    let block_stop = serde_json::json!({
        "type": "content_block_stop",
        "index": 0
    });
    append_sse(&mut buf, "content_block_stop", &block_stop.to_string());

    let msg_delta = serde_json::json!({
        "type": "message_delta",
        "delta": { "stop_reason": "end_turn" },
        "usage": { "output_tokens": usage.output_tokens }
    });
    append_sse(&mut buf, "message_delta", &msg_delta.to_string());

    append_sse(&mut buf, "message_stop", &serde_json::json!({"type":"message_stop"}).to_string());

    buf
}

/// Split text into chunks of approximately `size` bytes.
fn chunk_text(text: &str, size: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    text.as_bytes()
        .chunks(size)
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_frame_format() {
        let mut buf = String::new();
        append_sse(&mut buf, "message_start", r#"{"type":"message_start"}"#);
        assert!(buf.starts_with("event: message_start\n"));
        assert!(buf.contains("data: {\"type\":\"message_start\"}"));
        assert!(buf.ends_with("\n\n"));
    }

    #[test]
    fn streaming_text_contains_all_events() {
        let usage = Usage::new(50, 25);
        let stream = streaming_text("Hello, world!", &usage);
        assert!(stream.contains("event: message_start"));
        assert!(stream.contains("event: content_block_start"));
        assert!(stream.contains("event: content_block_delta"));
        assert!(stream.contains("text_delta"));
        assert!(stream.contains("event: content_block_stop"));
        assert!(stream.contains("event: message_delta"));
        assert!(stream.contains("end_turn"));
        assert!(stream.contains("event: message_stop"));
    }

    #[test]
    fn streaming_tool_use_contains_input_json_delta() {
        let usage = Usage::default();
        let stream = streaming_tool_use(
            "toolu_123",
            "read_file",
            r#"{"path":"test.txt"}"#,
            &usage,
        );
        assert!(stream.contains("input_json_delta"));
        assert!(stream.contains("tool_use"));
        assert!(stream.contains("toolu_123"));
        assert!(stream.contains("read_file"));
    }

    #[test]
    fn chunk_text_splits_correctly() {
        let chunks = chunk_text("abcdefghij", 3);
        assert_eq!(chunks, vec!["abc", "def", "ghi", "j"]);
    }
}
