# W1.1 — Instruction File Loader (DECIPHER.md)

Gap: G14 | Track: Runtime | Status: in progress

## Summary

Add automatic loading of `DECIPHER.md` instruction files into the system prompt,
giving DeCIpher project-specific context at zero per-session cost. Two-layer
resolution (user-level + workspace-level). Banner visibility. `/init` command
for scaffolding.

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| File name | `DECIPHER.md` only | Brand-consistent. Interop with `AGENTS.md`/`CLAUDE.md` deferred. |
| Layer count | 2 (user + workspace) | Matches skills/memory precedent. Subdirectory walking deferred. |
| Injection point | After Environment, before Memory | Project instructions are foundational context; memory/skills refine. |
| Approach | Standalone `instructions.rs` module | Parallels `skills.rs` and `memory.rs`. Testable in isolation. |
| Banner | Show loaded paths | Omit line entirely if no files found. |
| `/init` template | Static, no LLM | Fast, deterministic, works offline. |

## File Discovery

### Paths checked (in order)

1. `~/.decipher/DECIPHER.md` — user-level (global defaults)
2. `<workspace>/DECIPHER.md` — project-level (repo-specific)

Both are optional. If neither exists, no injection occurs.

### Merge Strategy

- **One file found:** Inject content directly under `## Project Instructions` with no sub-headers.
- **Both files found:** Concatenate with sub-headers:

```
## Project Instructions

### User Instructions (~/.decipher/DECIPHER.md)
<user-level content>

### Workspace Instructions (./DECIPHER.md)
<project-level content>
```

## System Prompt Injection

Injected in `build_system_prompt()` after the Environment section, before Memory:

```
... (Mission, Plan, Environment)

## Project Instructions
<merged content>

## Memory
<memory entries>

## Skills
<skill sections>

... (Instructions/behavioral)
```

## Banner Display

When instruction files are loaded, a line is added to the `ServerMessage::Banner`:

```
DeCIpher v0.4.0 | claude-3.5-sonnet | auto mode
Workspace: /home/user/my-project
Instructions: ~/.decipher/DECIPHER.md, ./DECIPHER.md
```

If no files found, the line is omitted entirely.

## `/init` Slash Command

**Trigger:** `/init` in TUI.

**Behavior:**
- If `<workspace>/DECIPHER.md` exists: show `DECIPHER.md already exists at <path>`, do nothing.
- If not: write starter template, show `Created <path>/DECIPHER.md`.

**Template:**

```markdown
# DECIPHER.md

## Project

<!-- What this project is and what DeCIpher should know about it -->

## Commands

<!-- Common commands DeCIpher should know -->
<!-- Example:
` ` `bash
# Build
make build

# Test
make test

# Deploy
make deploy
` ` `
-->

## Architecture

<!-- Key directories, modules, or patterns -->

## Rules

<!-- Conventions, constraints, or things to avoid -->
```

## Module Structure

**New file:** `crates/runtime/src/instructions.rs`

```rust
pub struct InstructionFiles {
    pub user_path: Option<PathBuf>,
    pub user_content: Option<String>,
    pub project_path: Option<PathBuf>,
    pub project_content: Option<String>,
}

/// Check both layers, read content from any that exist.
pub fn load_instructions(decipher_home: &Path, workspace: &Path) -> InstructionFiles

/// Build the `## Project Instructions` section for the system prompt.
pub fn format_instructions_section(files: &InstructionFiles) -> String

/// Write the starter DECIPHER.md template to workspace root.
/// Returns Ok(path) on success, Err if file already exists.
pub fn generate_template(workspace: &Path) -> Result<PathBuf, std::io::Error>
```

## Files Modified

| File | Change |
|------|--------|
| `crates/runtime/src/lib.rs` | Add `pub mod instructions;` |
| `crates/runtime/src/types.rs` | Add `pub instructions: InstructionFiles` to `AgentConfig` |
| `crates/runtime/src/agent_loop.rs` | Call `format_instructions_section()` in `build_system_prompt()` after Environment, before Memory |
| `crates/cli/src/main.rs` | Call `load_instructions()` in `build_agent_config()`. Add instruction paths to `Banner`. Handle `/init` slash command. |

## Tests

Unit tests in `instructions.rs` using `tempdir`:

1. No files exist → `format_instructions_section` returns empty string
2. User-level only → single block, no sub-headers
3. Project-level only → single block, no sub-headers
4. Both layers → merged output with sub-headers
5. `generate_template` → file created with expected content
6. `generate_template` when file already exists → returns error
