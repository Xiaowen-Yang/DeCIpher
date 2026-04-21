//! Token-aware context compaction.
//!
//! When the prompt token count approaches the model context window (75%),
//! compact older turns to keep the conversation within budget.
//!
//! Port source: `lib/compact.js` shouldCompact / compactMessages.
//!
//! The current implementation uses a simple sliding-window strategy:
//! keep the system setup message (index 0) + the most recent N messages.
//! A more sophisticated summarisation call (like compact.js) is planned for R3.

use decipher_providers::types::Message;

/// Returns true if the current prompt token count warrants compaction.
///
/// Threshold: 75% of the model context window.
/// `prompt_tokens` is the actual token count from the last API response.
pub fn should_compact(prompt_tokens: u32, context_window: u32) -> bool {
    if context_window == 0 || prompt_tokens == 0 {
        return false;
    }
    prompt_tokens as f64 / context_window as f64 >= 0.75
}

/// Compact a message list by removing middle turns.
///
/// Strategy:
/// - Keep the first message (initial user turn / mission setup).
/// - Insert a summary placeholder.
/// - Keep the most recent `keep_recent` messages.
///
/// Returns the compacted list.
pub fn compact_messages(messages: &[Message], keep_recent: usize) -> Vec<Message> {
    if messages.len() <= keep_recent + 2 {
        return messages.to_vec();
    }

    let first = messages[0].clone();
    let recent = messages[messages.len().saturating_sub(keep_recent)..].to_vec();
    let removed = messages.len() - keep_recent - 1;

    let summary = Message {
        role: "user".to_string(),
        content: decipher_providers::types::MessageContent::Text(format!(
            "[Earlier turns compacted — {removed} messages summarized]"
        )),
    };

    let mut result = vec![first, summary];
    result.extend(recent);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use decipher_providers::types::{Message, MessageContent};

    fn msg(role: &str, text: &str) -> Message {
        Message {
            role: role.to_string(),
            content: MessageContent::Text(text.to_string()),
        }
    }

    #[test]
    fn should_compact_at_75_percent() {
        assert!(should_compact(7500, 10000));
        assert!(should_compact(8000, 10000));
    }

    #[test]
    fn should_not_compact_below_threshold() {
        assert!(!should_compact(7000, 10000));
        assert!(!should_compact(0, 10000));
    }

    #[test]
    fn compact_keeps_first_and_recent() {
        let messages: Vec<Message> = (0..12)
            .map(|i| msg(if i % 2 == 0 { "user" } else { "assistant" }, &format!("msg{i}")))
            .collect();

        let compacted = compact_messages(&messages, 6);
        // first + summary + 6 recent = 8
        assert_eq!(compacted.len(), 8);
        // First message preserved.
        assert!(matches!(&compacted[0].content, MessageContent::Text(t) if t.contains("msg0")));
        // Summary injected.
        assert!(matches!(&compacted[1].content, MessageContent::Text(t) if t.contains("compacted")));
        // Last 6 messages preserved.
        assert!(matches!(&compacted[7].content, MessageContent::Text(t) if t.contains("msg11")));
    }

    #[test]
    fn compact_noop_when_short() {
        let messages: Vec<Message> = (0..5)
            .map(|i| msg("user", &format!("msg{i}")))
            .collect();
        let compacted = compact_messages(&messages, 6);
        assert_eq!(compacted.len(), messages.len());
    }
}
