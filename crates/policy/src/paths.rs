//! Path protection and workspace boundary enforcement.
//!
//! Ported from `isProtectedPath` / `isInWorkspace` in `lib/exec-policy.js`.

use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

// ── Protected path patterns ─────────────────────────────────────────────────

static PROTECTED_PATH_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"/\.git(?:/|$)", "/.git"),
        (r"/\.decipher(?:/|$)", "/.decipher"),
        (r"^/etc/", "/etc/"),
        (r"^/usr/", "/usr/"),
        (r"^/var/", "/var/"),
        (r"^/sys/", "/sys/"),
        (r"^/proc/", "/proc/"),
        (r"/\.ssh(?:/|$)", "/.ssh"),
        (r"/\.aws(?:/|$)", "/.aws"),
        (r"/\.kube(?:/|$)", "/.kube"),
        (r"/\.gnupg(?:/|$)", "/.gnupg"),
        (r"/\.env$", "/.env"),
        (r"/\.env\.", "/.env."),
    ]
    .iter()
    .map(|(pat, label)| (Regex::new(pat).expect("valid protected-path pattern"), *label))
    .collect()
});

// ── Public API ──────────────────────────────────────────────────────────────

/// Check if a path touches a protected location.
///
/// Returns `Some(pattern_label)` if the path is protected, `None` if safe.
/// `carveouts` are path prefixes that override protection (e.g., the workspace).
pub fn is_protected_path(path: &str, carveouts: &[&str]) -> Option<String> {
    if path.is_empty() {
        return None;
    }

    // Carveouts override protection
    for allowed in carveouts {
        if path.starts_with(allowed) {
            return None;
        }
    }

    for (pat, label) in PROTECTED_PATH_PATTERNS.iter() {
        if pat.is_match(path) {
            return Some(label.to_string());
        }
    }
    None
}

/// Check if a path is within the allowed workspace (or `/tmp`).
pub fn is_in_workspace(path: &str, workspace: &str) -> bool {
    if path.is_empty() || workspace.is_empty() {
        return false;
    }
    let resolved = canonicalize_best_effort(path, Some(workspace));
    let ws = canonicalize_best_effort(workspace, None);
    resolved.starts_with(&ws) || resolved.starts_with("/tmp")
}

/// Best-effort path normalization without requiring the path to exist.
///
/// Resolves relative paths against `base` (if provided), collapses `.` and `..`.
fn canonicalize_best_effort(path: &str, base: Option<&str>) -> String {
    let p = if path.starts_with('/') {
        Path::new(path).to_path_buf()
    } else if let Some(b) = base {
        Path::new(b).join(path)
    } else {
        Path::new(path).to_path_buf()
    };

    // Collapse components manually (std::fs::canonicalize requires the path to exist)
    let mut parts: Vec<&str> = Vec::new();
    for component in p.components() {
        match component {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::CurDir => {}
            std::path::Component::Normal(s) => {
                parts.push(s.to_str().unwrap_or(""));
            }
            std::path::Component::RootDir => {
                parts.clear();
                // Root will be re-added by the join below
            }
            std::path::Component::Prefix(_) => {}
        }
    }
    format!("/{}", parts.join("/"))
}

/// Resolve a path relative to a workspace (for policy evaluation).
///
/// Absolute paths are returned as-is after normalization.
/// Relative paths are resolved against the workspace.
pub fn resolve_path(path: &str, workspace: Option<&str>) -> String {
    if path.starts_with('/') {
        canonicalize_best_effort(path, None)
    } else if let Some(ws) = workspace {
        canonicalize_best_effort(path, Some(ws))
    } else {
        canonicalize_best_effort(path, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Protected paths ─────────────────────────────────────────────────

    #[test]
    fn git_dir_is_protected() {
        assert!(is_protected_path("/home/user/project/.git/config", &[]).is_some());
        assert!(is_protected_path("/app/.git/", &[]).is_some());
    }

    #[test]
    fn ssh_dir_is_protected() {
        assert!(is_protected_path("/home/user/.ssh/id_rsa", &[]).is_some());
    }

    #[test]
    fn env_file_is_protected() {
        assert!(is_protected_path("/app/.env", &[]).is_some());
        assert!(is_protected_path("/app/.env.production", &[]).is_some());
    }

    #[test]
    fn etc_is_protected() {
        assert!(is_protected_path("/etc/passwd", &[]).is_some());
    }

    #[test]
    fn normal_path_is_not_protected() {
        assert!(is_protected_path("/home/user/project/src/main.rs", &[]).is_none());
    }

    #[test]
    fn carveout_overrides_protection() {
        // .git inside the workspace is normally protected, but carveout allows it
        assert!(is_protected_path("/app/.git/config", &["/app"]).is_none());
    }

    #[test]
    fn empty_path_not_protected() {
        assert!(is_protected_path("", &[]).is_none());
    }

    // ── Workspace boundary ──────────────────────────────────────────────

    #[test]
    fn path_inside_workspace() {
        assert!(is_in_workspace("/home/user/project/src/main.rs", "/home/user/project"));
    }

    #[test]
    fn path_outside_workspace() {
        assert!(!is_in_workspace("/etc/passwd", "/home/user/project"));
    }

    #[test]
    fn tmp_always_allowed() {
        assert!(is_in_workspace("/tmp/build/output", "/home/user/project"));
    }

    #[test]
    fn relative_path_resolved_against_workspace() {
        assert!(is_in_workspace("src/main.rs", "/home/user/project"));
    }

    #[test]
    fn dotdot_escape_blocked() {
        // ../../../etc/passwd should resolve outside the workspace
        assert!(!is_in_workspace("../../../etc/passwd", "/home/user/project"));
    }

    // ── resolve_path ────────────────────────────────────────────────────

    #[test]
    fn resolve_absolute() {
        assert_eq!(resolve_path("/app/src/main.rs", None), "/app/src/main.rs");
    }

    #[test]
    fn resolve_relative_with_workspace() {
        assert_eq!(resolve_path("src/main.rs", Some("/app")), "/app/src/main.rs");
    }

    #[test]
    fn resolve_collapses_dotdot() {
        assert_eq!(resolve_path("/app/src/../lib/foo.rs", None), "/app/lib/foo.rs");
    }
}
