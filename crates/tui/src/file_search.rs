//! File search for @ mention popup.
//!
//! Walks the current directory (respecting .gitignore-like patterns),
//! performs fuzzy matching, and returns ranked results.

use std::path::Path;
use std::fs;

/// Maximum directory depth to walk.
const MAX_DEPTH: usize = 6;
/// Maximum results to return.
const MAX_RESULTS: usize = 20;
/// Directories to always skip.
const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", ".hg", ".svn", "target", "dist", "build",
    "__pycache__", ".tox", ".mypy_cache", ".pytest_cache", ".venv",
    "venv", ".next", ".nuxt", "coverage", ".cache",
];

/// A search result with path and match score.
#[derive(Debug, Clone)]
pub struct FileResult {
    /// Relative path from search root.
    pub path: String,
    /// Whether this is a directory.
    pub is_dir: bool,
    /// Whether this is an image file.
    pub is_image: bool,
    /// Fuzzy match score (higher = better).
    pub score: i32,
}

/// Search for files matching a query under the given root.
pub fn search_files(root: &Path, query: &str) -> Vec<FileResult> {
    let mut results = Vec::new();
    let query_lower = query.to_lowercase();

    walk_dir(root, root, &query_lower, &mut results, 0);

    // Sort by score descending, then by path length ascending
    results.sort_by(|a, b| {
        b.score.cmp(&a.score)
            .then(a.path.len().cmp(&b.path.len()))
    });

    results.truncate(MAX_RESULTS);
    results
}

fn walk_dir(
    root: &Path,
    dir: &Path,
    query: &str,
    results: &mut Vec<FileResult>,
    depth: usize,
) {
    if depth > MAX_DEPTH || results.len() >= MAX_RESULTS * 3 {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files/dirs (except common ones)
        if name.starts_with('.') && name != ".env" && name != ".env.example" {
            continue;
        }

        let is_dir = path.is_dir();

        // Skip excluded directories
        if is_dir && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }

        let rel_path = path.strip_prefix(root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| name.clone());

        let score = fuzzy_score(&rel_path, query);

        if score > 0 || query.is_empty() {
            let is_image = is_image_file(&name);
            results.push(FileResult {
                path: rel_path,
                is_dir,
                is_image,
                score: if query.is_empty() { 1 } else { score },
            });
        }

        if is_dir {
            walk_dir(root, &path, query, results, depth + 1);
        }
    }
}

/// Simple fuzzy matching score.
/// Returns 0 if no match, higher scores for better matches.
fn fuzzy_score(path: &str, query: &str) -> i32 {
    if query.is_empty() {
        return 1;
    }

    let path_lower = path.to_lowercase();

    // Exact substring match gets highest score
    if path_lower.contains(query) {
        let bonus = if path_lower.ends_with(query) { 20 } else { 0 };
        return 100 + bonus - path.len() as i32;
    }

    // Filename-only match
    let filename = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    if filename.contains(query) {
        return 80 - path.len() as i32;
    }

    // Fuzzy character match (all query chars appear in order)
    let mut query_chars = query.chars().peekable();
    let mut consecutive = 0;
    let mut max_consecutive = 0;
    let mut matched = 0;

    for ch in path_lower.chars() {
        if query_chars.peek() == Some(&ch) {
            query_chars.next();
            matched += 1;
            consecutive += 1;
            max_consecutive = max_consecutive.max(consecutive);
        } else {
            consecutive = 0;
        }
    }

    if query_chars.peek().is_none() {
        // All query chars matched
        30 + max_consecutive * 5 + matched - path.len() as i32
    } else {
        0
    }
}

fn is_image_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
        || lower.ends_with(".svg")
        || lower.ends_with(".bmp")
        || lower.ends_with(".ico")
}
