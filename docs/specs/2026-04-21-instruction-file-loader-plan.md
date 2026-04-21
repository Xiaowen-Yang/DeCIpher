# W1.1 Instruction File Loader — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Load `DECIPHER.md` instruction files from user-level and workspace-level paths, inject into the system prompt, show loaded paths in the banner, and provide `/init` to scaffold a template.

**Architecture:** New `crates/runtime/src/instructions.rs` module with `InstructionFiles` struct, loader, formatter, and template generator. Wired into `AgentConfig`, `build_system_prompt()`, `build_agent_config()`, `banner_lines()`, and the slash command handler.

**Tech Stack:** Rust, std::fs, tempfile (tests)

---

### Task 1: Create `instructions.rs` — struct + loader + tests

**Files:**
- Create: `crates/runtime/src/instructions.rs`

- [ ] **Step 1: Write the failing test for "no files → empty struct"**

Add to `crates/runtime/src/instructions.rs`:

```rust
//! Instruction file loader for DeCIpher.
//!
//! Loads `DECIPHER.md` from two layers:
//!  1. `~/.decipher/DECIPHER.md` — user-level (global defaults)
//!  2. `<workspace>/DECIPHER.md` — project-level (repo-specific)

use std::path::{Path, PathBuf};

/// Loaded instruction files from user and project layers.
#[derive(Debug, Clone, Default)]
pub struct InstructionFiles {
    pub user_path: Option<PathBuf>,
    pub user_content: Option<String>,
    pub project_path: Option<PathBuf>,
    pub project_content: Option<String>,
}

impl InstructionFiles {
    /// Returns true if no instruction files were found.
    pub fn is_empty(&self) -> bool {
        self.user_content.is_none() && self.project_content.is_none()
    }

    /// Returns display-friendly list of loaded paths (for banner).
    pub fn loaded_paths_display(&self) -> Option<String> {
        let mut paths = Vec::new();
        if let Some(ref p) = self.user_path {
            paths.push(format!("~/.decipher/DECIPHER.md"));
        }
        if let Some(ref p) = self.project_path {
            paths.push(format!("./DECIPHER.md"));
        }
        if paths.is_empty() {
            None
        } else {
            Some(paths.join(", "))
        }
    }
}

/// Load instruction files from user home and workspace directories.
pub fn load_instructions(decipher_home: &Path, workspace: &Path) -> InstructionFiles {
    let mut files = InstructionFiles::default();

    // User-level: ~/.decipher/DECIPHER.md
    let user_file = decipher_home.join("DECIPHER.md");
    if user_file.is_file() {
        if let Ok(content) = std::fs::read_to_string(&user_file) {
            let content = content.trim().to_string();
            if !content.is_empty() {
                files.user_path = Some(user_file);
                files.user_content = Some(content);
            }
        }
    }

    // Project-level: <workspace>/DECIPHER.md
    let project_file = workspace.join("DECIPHER.md");
    if project_file.is_file() {
        if let Ok(content) = std::fs::read_to_string(&project_file) {
            let content = content.trim().to_string();
            if !content.is_empty() {
                files.project_path = Some(project_file);
                files.project_content = Some(content);
            }
        }
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_files_returns_empty() {
        let files = load_instructions(Path::new("/nonexistent"), Path::new("/also_nonexistent"));
        assert!(files.is_empty());
        assert!(files.loaded_paths_display().is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p decipher-runtime -- instructions::tests::no_files_returns_empty`
Expected: PASS

- [ ] **Step 3: Write the failing test for "user-level only"**

Add to the `tests` module in `instructions.rs`:

```rust
    #[test]
    fn user_level_only() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("DECIPHER.md"), "# User instructions\nBe concise.").unwrap();

        let files = load_instructions(home.path(), ws.path());
        assert!(!files.is_empty());
        assert!(files.user_content.is_some());
        assert!(files.project_content.is_none());
        assert_eq!(files.loaded_paths_display().unwrap(), "~/.decipher/DECIPHER.md");
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p decipher-runtime -- instructions::tests::user_level_only`
Expected: PASS

- [ ] **Step 5: Write the failing test for "project-level only"**

```rust
    #[test]
    fn project_level_only() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("DECIPHER.md"), "# Project rules\nUse make.").unwrap();

        let files = load_instructions(home.path(), ws.path());
        assert!(!files.is_empty());
        assert!(files.user_content.is_none());
        assert!(files.project_content.is_some());
        assert_eq!(files.loaded_paths_display().unwrap(), "./DECIPHER.md");
    }
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test -p decipher-runtime -- instructions::tests::project_level_only`
Expected: PASS

- [ ] **Step 7: Write the failing test for "both layers"**

```rust
    #[test]
    fn both_layers_loaded() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("DECIPHER.md"), "# User defaults").unwrap();
        std::fs::write(ws.path().join("DECIPHER.md"), "# Project rules").unwrap();

        let files = load_instructions(home.path(), ws.path());
        assert!(!files.is_empty());
        assert!(files.user_content.is_some());
        assert!(files.project_content.is_some());
        assert_eq!(files.loaded_paths_display().unwrap(), "~/.decipher/DECIPHER.md, ./DECIPHER.md");
    }
```

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo test -p decipher-runtime -- instructions::tests::both_layers_loaded`
Expected: PASS

- [ ] **Step 9: Write test for empty file is ignored**

```rust
    #[test]
    fn empty_file_is_ignored() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("DECIPHER.md"), "   \n  ").unwrap();

        let files = load_instructions(home.path(), ws.path());
        assert!(files.is_empty());
    }
```

- [ ] **Step 10: Run all instruction tests**

Run: `cargo test -p decipher-runtime -- instructions::tests`
Expected: all 5 PASS

- [ ] **Step 11: Commit**

```bash
git add crates/runtime/src/instructions.rs
git commit -m "feat(runtime): add instructions.rs — DECIPHER.md loader with 5 tests"
```

---

### Task 2: Add `format_instructions_section` + tests

**Files:**
- Modify: `crates/runtime/src/instructions.rs`

- [ ] **Step 1: Write the failing test for single-layer formatting**

Add to `instructions.rs` above the tests module:

```rust
/// Format loaded instructions into a system prompt section.
///
/// - If no files loaded: returns empty string.
/// - If one layer: `## Project Instructions\n<content>` (no sub-headers).
/// - If both layers: sub-headers for each layer.
pub fn format_instructions_section(files: &InstructionFiles) -> String {
    if files.is_empty() {
        return String::new();
    }

    let both = files.user_content.is_some() && files.project_content.is_some();

    let mut out = String::from("## Project Instructions\n\n");

    if let Some(ref content) = files.user_content {
        if both {
            out.push_str("### User Instructions (~/.decipher/DECIPHER.md)\n");
        }
        out.push_str(content);
        out.push_str("\n\n");
    }

    if let Some(ref content) = files.project_content {
        if both {
            out.push_str("### Workspace Instructions (./DECIPHER.md)\n");
        }
        out.push_str(content);
        out.push_str("\n\n");
    }

    out.trim_end().to_string()
}
```

Add test:

```rust
    #[test]
    fn format_single_layer_no_subheaders() {
        let files = InstructionFiles {
            user_path: None,
            user_content: None,
            project_path: Some(PathBuf::from("./DECIPHER.md")),
            project_content: Some("# Rules\nUse cargo test.".to_string()),
        };
        let section = format_instructions_section(&files);
        assert!(section.contains("## Project Instructions"));
        assert!(section.contains("# Rules\nUse cargo test."));
        assert!(!section.contains("### Workspace Instructions"));
    }
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p decipher-runtime -- instructions::tests::format_single_layer_no_subheaders`
Expected: PASS

- [ ] **Step 3: Write test for both layers with sub-headers**

```rust
    #[test]
    fn format_both_layers_with_subheaders() {
        let files = InstructionFiles {
            user_path: Some(PathBuf::from("/home/.decipher/DECIPHER.md")),
            user_content: Some("Be concise.".to_string()),
            project_path: Some(PathBuf::from("./DECIPHER.md")),
            project_content: Some("Use make.".to_string()),
        };
        let section = format_instructions_section(&files);
        assert!(section.contains("## Project Instructions"));
        assert!(section.contains("### User Instructions (~/.decipher/DECIPHER.md)"));
        assert!(section.contains("Be concise."));
        assert!(section.contains("### Workspace Instructions (./DECIPHER.md)"));
        assert!(section.contains("Use make."));
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p decipher-runtime -- instructions::tests::format_both_layers_with_subheaders`
Expected: PASS

- [ ] **Step 5: Write test for empty returns empty string**

```rust
    #[test]
    fn format_empty_returns_empty() {
        let files = InstructionFiles::default();
        assert!(format_instructions_section(&files).is_empty());
    }
```

- [ ] **Step 6: Run all instruction tests**

Run: `cargo test -p decipher-runtime -- instructions::tests`
Expected: all 8 PASS

- [ ] **Step 7: Commit**

```bash
git add crates/runtime/src/instructions.rs
git commit -m "feat(runtime): add format_instructions_section with 3 formatter tests"
```

---

### Task 3: Add `generate_template` + tests

**Files:**
- Modify: `crates/runtime/src/instructions.rs`

- [ ] **Step 1: Write the template generator function and failing test**

Add to `instructions.rs`:

```rust
/// The starter DECIPHER.md template content.
const TEMPLATE: &str = r#"# DECIPHER.md

## Project

<!-- What this project is and what DeCIpher should know about it -->

## Commands

<!-- Common commands DeCIpher should know -->
<!-- Example:
```bash
# Build
make build

# Test
make test

# Deploy
make deploy
```
-->

## Architecture

<!-- Key directories, modules, or patterns -->

## Rules

<!-- Conventions, constraints, or things to avoid -->
"#;

/// Write the starter DECIPHER.md template to workspace root.
///
/// Returns `Ok(path)` on success. Returns `Err` if file already exists.
pub fn generate_template(workspace: &Path) -> Result<PathBuf, std::io::Error> {
    let target = workspace.join("DECIPHER.md");
    if target.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("DECIPHER.md already exists at {}", target.display()),
        ));
    }
    std::fs::write(&target, TEMPLATE)?;
    Ok(target)
}
```

Add tests:

```rust
    #[test]
    fn generate_template_creates_file() {
        let ws = tempfile::tempdir().unwrap();
        let path = generate_template(ws.path()).unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# DECIPHER.md"));
        assert!(content.contains("## Project"));
        assert!(content.contains("## Commands"));
        assert!(content.contains("## Architecture"));
        assert!(content.contains("## Rules"));
    }

    #[test]
    fn generate_template_fails_if_exists() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("DECIPHER.md"), "existing").unwrap();
        let result = generate_template(ws.path());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    }
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p decipher-runtime -- instructions::tests::generate_template`
Expected: both PASS

- [ ] **Step 3: Run all instruction tests**

Run: `cargo test -p decipher-runtime -- instructions::tests`
Expected: all 10 PASS

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/instructions.rs
git commit -m "feat(runtime): add generate_template for /init scaffolding, 2 tests"
```

---

### Task 4: Wire module into runtime crate

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `crates/runtime/src/types.rs`

- [ ] **Step 1: Register module in `lib.rs`**

In `crates/runtime/src/lib.rs`, add after `pub mod hooks;`:

```rust
pub mod instructions;
```

And add to the `pub use` block:

```rust
pub use instructions::{InstructionFiles, load_instructions, format_instructions_section, generate_template};
```

- [ ] **Step 2: Add `instructions` field to `AgentConfig` in `types.rs`**

In `crates/runtime/src/types.rs`, add the import at the top:

```rust
use crate::instructions::InstructionFiles;
```

Add field after `memory_context`:

```rust
    /// Loaded instruction files (DECIPHER.md) for system prompt injection.
    pub instructions: InstructionFiles,
```

In the `Default` impl, add:

```rust
            instructions: InstructionFiles::default(),
```

- [ ] **Step 3: Inject into `build_system_prompt()` in `agent_loop.rs`**

In `crates/runtime/src/agent_loop.rs`, add after the Environment section (line 841) and before the Memory section (line 843):

```rust
    // Inject project instructions (DECIPHER.md) if available.
    let instructions_section = crate::instructions::format_instructions_section(&config.instructions);
    if !instructions_section.is_empty() {
        prompt.push_str(&instructions_section);
        prompt.push_str("\n\n");
    }
```

- [ ] **Step 4: Verify build compiles**

Run: `cargo build -p decipher-runtime`
Expected: compiles with no errors

- [ ] **Step 5: Run all runtime tests**

Run: `cargo test -p decipher-runtime`
Expected: all pass (existing + new instruction tests)

- [ ] **Step 6: Commit**

```bash
git add crates/runtime/src/lib.rs crates/runtime/src/types.rs crates/runtime/src/agent_loop.rs
git commit -m "feat(runtime): wire instructions module into AgentConfig and system prompt"
```

---

### Task 5: Wire into CLI — loader, banner, `/init` command

**Files:**
- Modify: `crates/cli/src/main.rs`
- Modify: `crates/tui/src/bottom_pane.rs`

- [ ] **Step 1: Load instructions in `build_agent_config()`**

In `crates/cli/src/main.rs`, in `build_agent_config()` (line ~1090), after the `load_skills` call, add:

```rust
    let instructions = load_instructions(decipher_home, std::path::Path::new(&workspace));
```

Add to the `AgentConfig` struct construction:

```rust
        instructions,
```

Add the import near the top of the file where other runtime imports are:

```rust
use decipher_runtime::{load_instructions, generate_template};
```

- [ ] **Step 2: Add instruction paths to banner**

In `crates/tui/src/bottom_pane.rs`, modify `banner_lines()` signature to accept an optional instructions display string:

```rust
pub fn banner_lines(
    version: &str,
    provider: &str,
    model: &str,
    directory: &str,
    api_key_set: bool,
    instructions_display: Option<&str>,
) -> Vec<Line<'static>> {
```

After the `directory` line (around line 576), before the `approval` line, add:

```rust
    if let Some(instr) = instructions_display {
        lines.push(Line::from(vec![
            Span::styled("  instruct.  ", DIM),
            Span::styled(instr.to_string(), CYAN),
        ]));
    }
```

Update all call sites of `banner_lines()` in `crates/cli/src/main.rs` to pass the instructions display. There are two call sites (line ~228 initial banner, and line ~765 on Banner event). At both sites, compute the display string from the loaded `InstructionFiles`:

For the initial banner (line ~228), the `instructions` value needs to be available. Store it as a local variable before building the agent config, or load it early:

```rust
let instructions = load_instructions(&decipher_home, std::path::Path::new(&workspace));
let instr_display = instructions.loaded_paths_display();
// ... pass to banner_lines:
let banner = bottom_pane::banner_lines(version, provider, &model, &directory, api_key_set, instr_display.as_deref());
```

- [ ] **Step 3: Handle `/init` slash command**

In the slash command handler section of `crates/cli/src/main.rs` (around line ~585, after the `/resume` handler or before the else-fall-through), add:

```rust
                                        } else if text_trimmed == "/init" {
                                            // ── /init ─────────────────────────────────────────
                                            let text = match generate_template(std::path::Path::new(&workspace)) {
                                                Ok(path) => format!("Created {}", path.display()),
                                                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                                                    format!("DECIPHER.md already exists at {}/DECIPHER.md", workspace)
                                                }
                                                Err(e) => format!("Failed to create DECIPHER.md: {}", e),
                                            };
                                            let _ = event_tx.try_send(ServerMessage::AgentMessage { text });
```

- [ ] **Step 4: Verify full build compiles**

Run: `cargo build`
Expected: compiles with no errors

- [ ] **Step 5: Run all tests**

Run: `cargo test`
Expected: all pass

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/main.rs crates/tui/src/bottom_pane.rs
git commit -m "feat(cli): wire instruction loader into banner + add /init command"
```

---

### Task 6: Update docs

**Files:**
- Modify: `docs/v4/COMMANDS.md`
- Modify: `docs/v4/ARCHITECTURE.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add `/init` to COMMANDS.md**

Add `/init` to the slash command table with description: "Generate a starter DECIPHER.md in the workspace root."

- [ ] **Step 2: Add instruction files to ARCHITECTURE.md extension surface**

In the Extension Surface section, add an entry for Instructions:

```markdown
Instructions (W1.1)
  ~/.decipher/DECIPHER.md  <- user-level (global defaults)
  <workspace>/DECIPHER.md  <- project-level (repo-specific)
  Both layers merged. Injected as "## Project Instructions" in system prompt.
  /init generates a starter template.
```

- [ ] **Step 3: Add DECIPHER.md to CLAUDE.md repository map**

In the Repository Map section, note that `DECIPHER.md` at workspace root is loaded automatically.

- [ ] **Step 4: Update v4-todo.md — mark W1.1 as done**

Change W1.1 status from `**in progress**` to `**done**` with test count.

- [ ] **Step 5: Commit**

```bash
git add docs/v4/COMMANDS.md docs/v4/ARCHITECTURE.md CLAUDE.md docs/tasks/v4-todo.md
git commit -m "docs: add instruction file loader to commands, architecture, and CLAUDE.md"
```
