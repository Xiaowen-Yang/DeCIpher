# DeCIpher CLI Surface (V4)

This document owns user-visible CLI and TUI behavior. Roadmap sequencing lives
in `docs/tasks/v4-todo.md`.

## Principles

- Natural language starts execution by default.
- Slash commands control the current session.
- Document only what the code actually exposes today.
- Planned commands and flags should point back to a backlog owner epic.

## CLI Modes

### Interactive (current default)

```bash
decipher
decipher "fix the CI"
```

### Non-Interactive (planned, owned by P1.5)

```bash
decipher exec "task"
cat file | decipher exec "analyze"
decipher exec --output-format json "task"
```

## Flags

| Flag | Status | Notes |
|------|--------|-------|
| `--model <name>` | Implemented | Set active model (env: DECIPHER_MODEL) |
| `--resume [id]` | Implemented | Resume a saved session by thread_id or most recent |
| `--trust` | Planned (`P0.2`) | Full-access approval mode |
| `--read-only` | Planned (`P0.2`) | Browse only, no writes or destructive tools |
| `--plan` | Planned (`P1.6`) | Start in review-before-execute mode |
| `--output-format <text|json>` | Planned (`P1.5`) | Structured output for non-interactive mode |
| `--quiet` | Planned (`P1.5`) | Suppress progress in non-interactive mode |
| `--worktree` | Planned (`P3.2`) | Start in an isolated git worktree |

## Primary Behavior

Natural-language input triggers the execution pipeline:
1. Understand the goal (LLM system prompt analysis)
2. Execute tools in the agent loop (max 20 turns)
3. Verify the result
4. Report `PASS`, `FAIL`, or `PARTIAL`

## Slash Command Surface

### TUI-Local Commands

Intercepted in the Rust TUI before reaching the runtime.

| Command | Current Semantics |
|---------|-------------------|
| `/clear` | Clear local scrollback |
| `/copy` | Copy the last agent response to clipboard |
| `/resume [id]` | Resume saved session by thread_id; no arg = most recent |

### Server-Advertised Commands

Exposed through `SLASH_COMMANDS` in `lib/cli-surface.js`.

| Command | Current Semantics |
|---------|-------------------|
| `/help` | Show command help and usage |
| `/model` | Show or change the active model |
| `/setting` | Show or update configuration |
| `/status` | Show current session status |
| `/plan` | Show the current mission plan (view only, not review-before-execute) |
| `/review` | Show current repair/session snapshot |
| `/transcript` | Show transcript tail |
| `/log` | Alias for `/transcript` |
| `/artifacts` | Show saved artifacts and workspace info |
| `/demo` | Run a demo scenario |
| `/compact` | Trigger context compaction immediately |
| `/doctor` | Run environment health checks |
| `/agents` | List agent directories and registered tools |
| `/quit` | Exit DeCIpher |

### Planned Commands

| Command | Owner Epic | Intended Behavior |
|---------|------------|-------------------|
| `/sessions` | `P1.3` | List saved sessions and browse/resume |
| `/policy` | `P0.2` | Show or change approval policy |
| `/skills` | `P1.4` | List available skills/custom commands |
| `/hooks` | `P1.2` | List registered lifecycle hooks |
| `/mcp` | `P2.1` | List MCP servers and exposed tools |
| `/memory` | `P2.2` | Browse persistent memory entries |
| `/init` | W1.1 (done) | Generate starter `DECIPHER.md` in workspace root |
| `/diff` | `P3.4` | Show reviewable git changes |
| `/fork` | `P3.5` | Fork the current session |

## TUI Input And Controls

### Editing

- Multi-line editing and bracketed paste
- Emacs-style keys: `Ctrl+A/E` (start/end), `Ctrl+K/U` (kill), `Ctrl+Y` (yank)
- Word navigation: `Alt+B/F`, `Ctrl+W` (kill word), `Alt+D` (kill word forward)
- External editor: `Ctrl+X`

### Navigation

- `/` opens the command popup (fuzzy filter)
- `@` opens file search
- `Ctrl+R` opens reverse history search
- `Ctrl+T` opens the transcript pager (j/k scroll, q quit)
- `?` toggles the shortcuts overlay

### Session Actions

- `Enter` submits input
- `Tab` submits or queues when the agent is busy
- `Ctrl+C` interrupts the agent (first press) or exits (second press within 2s)
- `Ctrl+V` pastes an image from the clipboard
- `Ctrl+Z` suspends the app
- `Shift+Enter` inserts a newline

### Approval Overlay

- `y` or `Enter` approves the current request
- `a` approves all for the current session (always-approve mode)
- `n` or `Esc` denies the current request

## Developer Notes

Command semantics depend on runtime work tracked elsewhere:

| Command Area | Runtime Dependency | Owning Backlog |
|--------------|--------------------|----------------|
| `/resume` | Session store + JSONL load/resume | R3 (done) |
| `/sessions` | Session browser TUI | P1.3 |
| `/plan`, `--plan` | Review-before-execute gate | P1.6 |
| `/policy`, `--trust`, `--read-only` | Policy modes | P0.2 (partially done) |
| `/hooks`, `/skills`, `/mcp` | Extension surface | P1.2, P1.4, P2.1 |
| `/init` | Instruction file scaffolding | W1.1 (done) |
| `decipher exec`, `--output-format` | Non-interactive mode | P1.5 |
