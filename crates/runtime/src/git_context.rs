//! Git context collector for DeCIpher.
//!
//! Collects the current branch, HEAD SHA, and dirty file count from the
//! workspace's Git repository and formats them for injection into the agent
//! system prompt.

use std::path::Path;
use std::process::{Command, Stdio};

/// Snapshot of the current Git repository state for the workspace.
#[derive(Debug, Clone)]
pub struct GitContext {
    pub branch: String,
    pub head_sha: String,
    pub dirty_count: usize,
}

/// Collect Git context from the workspace directory.
///
/// Runs three Git commands against `workspace`:
///  - `git rev-parse --short HEAD` → `head_sha`
///  - `git rev-parse --abbrev-ref HEAD` → `branch` (`"HEAD"` when detached)
///  - `git status --porcelain` → `dirty_count` (non-empty output lines)
///
/// Returns `None` if the directory is not inside a Git repository or if any
/// command fails.
pub fn collect_git_context(workspace: &Path) -> Option<GitContext> {
    let head_sha = run_git(workspace, &["rev-parse", "--short", "HEAD"])?;
    let branch = run_git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let status_output = run_git(workspace, &["status", "--porcelain"])?;

    let dirty_count = status_output
        .lines()
        .filter(|l| !l.is_empty())
        .count();

    Some(GitContext {
        branch,
        head_sha,
        dirty_count,
    })
}

/// Format a `GitContext` into lines suitable for a system prompt.
///
/// Output always includes the branch and HEAD SHA. The dirty-files line is
/// appended only when `dirty_count > 0`.
pub fn format_git_lines(ctx: &GitContext) -> String {
    let mut out = format!("Git branch: {}\nGit HEAD: {}", ctx.branch, ctx.head_sha);
    if ctx.dirty_count > 0 {
        out.push_str(&format!("\nDirty files: {}", ctx.dirty_count));
    }
    out
}

/// Run a Git command in `workspace` and return trimmed stdout, or `None` on
/// failure (non-zero exit code, missing binary, etc.).
fn run_git(workspace: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Stdio};

    /// Initialise a minimal Git repo in `dir` (no commits yet).
    fn init_git_repo(dir: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git init failed");

        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git config user.email failed");

        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git config user.name failed");
    }

    /// Write a file, stage it, and create a commit in `dir`.
    fn make_commit(dir: &Path, filename: &str, msg: &str) {
        fs::write(dir.join(filename), "content").expect("write file failed");

        Command::new("git")
            .args(["add", filename])
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git add failed");

        Command::new("git")
            .args(["commit", "-m", msg])
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git commit failed");
    }

    #[test]
    fn collect_in_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        init_git_repo(dir);
        make_commit(dir, "README.md", "initial commit");

        let ctx = collect_git_context(dir).expect("expected Some from git repo");

        assert!(!ctx.branch.is_empty(), "branch should be non-empty");
        assert_eq!(ctx.head_sha.len(), 7, "short SHA should be 7 chars");
        assert_eq!(ctx.dirty_count, 0, "clean repo should have dirty_count 0");
    }

    #[test]
    fn collect_in_non_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = collect_git_context(tmp.path());
        assert!(result.is_none(), "plain directory should return None");
    }

    #[test]
    fn collect_dirty_count() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        init_git_repo(dir);
        make_commit(dir, "README.md", "initial commit");

        // Write an untracked file — shows up in `git status --porcelain`.
        fs::write(dir.join("dirty.txt"), "uncommitted").unwrap();

        let ctx = collect_git_context(dir).expect("expected Some from git repo");
        assert_eq!(ctx.dirty_count, 1, "one untracked file → dirty_count 1");
    }

    #[test]
    fn format_clean_repo() {
        let ctx = GitContext {
            branch: "main".to_string(),
            head_sha: "abc1234".to_string(),
            dirty_count: 0,
        };

        let output = format_git_lines(&ctx);

        assert!(output.contains("Git branch: main"), "should contain branch line");
        assert!(output.contains("Git HEAD: abc1234"), "should contain HEAD line");
        assert!(
            !output.contains("Dirty files"),
            "clean repo should not have dirty line"
        );
    }

    #[test]
    fn format_dirty_repo() {
        let ctx = GitContext {
            branch: "feature/x".to_string(),
            head_sha: "deadbee".to_string(),
            dirty_count: 3,
        };

        let output = format_git_lines(&ctx);

        assert!(output.contains("Git branch: feature/x"), "should contain branch line");
        assert!(output.contains("Git HEAD: deadbee"), "should contain HEAD line");
        assert!(output.contains("Dirty files: 3"), "should contain dirty line with count");
    }
}
