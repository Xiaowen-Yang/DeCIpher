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

// ── RowBuilder ──────────────────────────────────────────────────────────────

/// Stateful row accumulator that wraps text to a target width.
///
/// Caches results by (text, width) — rebuilds only when the inputs change.
/// Useful inside Cell implementations that need to re-render on resize without
/// re-running the wrapping algorithm on every frame.
///
/// ```rust,ignore
/// let mut builder = RowBuilder::new(80);
/// builder.push("Hello world, this is a long sentence that may wrap.");
/// let rows = builder.rows();
/// ```
pub struct RowBuilder {
    target_width: usize,
    rows: Vec<String>,
    /// Fingerprint of the last text passed to `push()` + width.
    cache_key: u64,
}

impl RowBuilder {
    pub fn new(width: usize) -> Self {
        Self {
            target_width: width,
            rows: Vec::new(),
            cache_key: 0,
        }
    }

    /// Set a new target width. Invalidates cache.
    pub fn set_width(&mut self, width: usize) {
        if self.target_width != width {
            self.target_width = width;
            self.rows.clear();
            self.cache_key = 0;
        }
    }

    /// Rebuild rows from `text` if the (text, width) pair changed.
    /// Returns true if the cache was rebuilt.
    pub fn push(&mut self, text: &str) -> bool {
        let key = hash_text_width(text, self.target_width);
        if key == self.cache_key && !self.rows.is_empty() {
            return false;
        }
        self.rows = wrap_text(text, self.target_width);
        self.cache_key = key;
        true
    }

    /// The current wrapped rows.
    pub fn rows(&self) -> &[String] {
        &self.rows
    }

    /// Number of rows produced by the last `push()`.
    pub fn height(&self) -> usize {
        self.rows.len().max(1)
    }
}

fn hash_text_width(text: &str, width: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    struct SimpleHasher(u64);
    impl Hasher for SimpleHasher {
        fn finish(&self) -> u64 { self.0 }
        fn write(&mut self, bytes: &[u8]) {
            for &b in bytes {
                self.0 = self.0.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(b as u64);
            }
        }
    }
    let mut h = SimpleHasher(0xcbf29ce484222325);
    text.hash(&mut h);
    width.hash(&mut h);
    h.finish()
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

    #[test]
    fn row_builder_caches() {
        let mut b = RowBuilder::new(20);
        let rebuilt = b.push("hello world this is text");
        assert!(rebuilt);
        let rows1 = b.rows().to_vec();
        // Push the same text again — should not rebuild
        let rebuilt2 = b.push("hello world this is text");
        assert!(!rebuilt2);
        assert_eq!(b.rows(), rows1.as_slice());
    }

    #[test]
    fn row_builder_rebuilds_on_text_change() {
        let mut b = RowBuilder::new(20);
        b.push("first text");
        b.push("different text here");
        assert!(b.rows().iter().any(|r| r.contains("different")));
    }

    #[test]
    fn row_builder_rebuilds_on_width_change() {
        let mut b = RowBuilder::new(80);
        b.push("some long text that wraps differently at narrow widths");
        let wide_rows = b.rows().len();
        b.set_width(20);
        b.push("some long text that wraps differently at narrow widths");
        let narrow_rows = b.rows().len();
        assert!(narrow_rows >= wide_rows);
    }
}
