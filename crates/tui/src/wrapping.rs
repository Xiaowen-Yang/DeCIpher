//! URL-aware text wrapping.
//!
//! Wraps text at a given width, treating URL-like tokens as unbreakable.
//! Non-URL words can be broken at character boundaries if they exceed width.
//!
//! Codex ref: `codex-rs/tui/src/wrapping.rs`

/// Wrap text to the given column width, preserving URLs.
/// Returns a Vec of wrapped lines.
pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();

    for input_line in text.split('\n') {
        if input_line.is_empty() {
            result.push(String::new());
            continue;
        }

        let has_urls = contains_url_like(input_line);

        if !has_urls {
            // Standard wrapping: break at spaces, long words at char boundary
            wrap_standard(input_line, width, &mut result);
        } else {
            // URL-aware: never break URL tokens
            wrap_url_aware(input_line, width, &mut result);
        }
    }

    result
}

fn wrap_standard(line: &str, width: usize, out: &mut Vec<String>) {
    let mut current = String::new();
    let mut col = 0;

    for word in line.split_inclusive(' ') {
        let word_len = word.chars().count();

        if col + word_len <= width || col == 0 {
            current.push_str(word);
            col += word_len;
        } else {
            out.push(current.trim_end().to_string());
            current = word.to_string();
            col = word_len;
        }
    }

    if !current.is_empty() {
        out.push(current.trim_end().to_string());
    }
}

fn wrap_url_aware(line: &str, width: usize, out: &mut Vec<String>) {
    let mut current = String::new();
    let mut col = 0;

    for token in line.split_inclusive(' ') {
        let token_len = token.trim().chars().count();
        let is_url = is_url_like(token.trim());

        if is_url {
            // Never break URLs — put on current line if it fits, else new line
            if col > 0 && col + token_len > width {
                out.push(current.trim_end().to_string());
                current = token.to_string();
                col = token_len;
            } else {
                current.push_str(token);
                col += token.chars().count();
            }
        } else {
            let word_len = token.chars().count();
            if col + word_len <= width || col == 0 {
                current.push_str(token);
                col += word_len;
            } else {
                out.push(current.trim_end().to_string());
                current = token.to_string();
                col = word_len;
            }
        }
    }

    if !current.is_empty() {
        out.push(current.trim_end().to_string());
    }
}

/// Check if a line contains any URL-like token.
fn contains_url_like(line: &str) -> bool {
    line.split_whitespace().any(|w| is_url_like(w))
}

/// Check if a token looks like a URL.
fn is_url_like(token: &str) -> bool {
    token.starts_with("http://")
        || token.starts_with("https://")
        || token.starts_with("file://")
        || token.starts_with("ftp://")
        || token.starts_with("ssh://")
        || token.contains("://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_wrap() {
        let result = wrap_text("hello world this is a test", 12);
        assert_eq!(result, vec!["hello world", "this is a", "test"]);
    }

    #[test]
    fn test_url_preserved() {
        let result = wrap_text("see https://example.com/very/long/path/here for details", 20);
        // URL should not be broken
        assert!(result.iter().any(|l| l.contains("https://example.com/very/long/path/here")));
    }

    #[test]
    fn test_empty() {
        let result = wrap_text("", 80);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn test_multiline() {
        let result = wrap_text("line one\nline two", 80);
        assert_eq!(result, vec!["line one", "line two"]);
    }
}
