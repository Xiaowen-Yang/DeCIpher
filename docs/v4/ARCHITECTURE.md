# DeCIpher System Architecture (V4)

This document owns the current technical shape of the system and the target
architectural direction. Delivery order lives in `docs/tasks/v4-todo.md`.

## Current Architecture

### Implementation Language Reality

DeCIpher is a Rust-native runtime. The main execution path is entirely Rust:

- Rust owns the TUI, terminal state, rendering, protocol types
- Rust owns the agent loop, tool dispatch, compaction, policy evaluation
- Rust owns the provider calls (Anthropic SSE streaming, tool calling)
- Rust owns session persistence (JSONL recording, resume)

Remaining JS:
- `bin/decipher` — thin shim (~160 lines) that execs the Rust binary
- `lib/api-client.js` — used by secondary agents (triage/fixer), pending R5
- `agents/planner/`, `agents/triage/`, `agents/fixer/`, `agents/verifier/` — secondary agents, pending R5

### Runtime Stack

```
bin/decipher (JS shim)
  -> exec target/release/decipher-tui

crates/cli/src/main.rs
  -> CliConfig::load() (env vars + ~/.decipher/config.json)
  -> AnthropicProvider::new(api_key, model)
  -> tokio::spawn(AgentLoop::run(config, provider, event_tx, approval_rx))
  -> terminal event loop (crossterm EventStream)
  -> event_rx.recv() for ServerMessage from agent
  -> approval_tx.send(bool) for approval decisions

crates/runtime/src/agent_loop.rs
  -> build system prompt + tool definitions
  -> for turn 1..max_turns:
     -> stream from provider (SSE + InputJsonDelta reconstruction)
     -> parse tool_use content blocks
     -> evaluate policy (allow/deny/ask) per tool
     -> dispatch tool via crates/runtime/src/tools/
     -> emit events (ToolStart, ToolResult, FilesModified, AgentStatus)
     -> compact if context > 75% of window
  -> emit MissionComplete

crates/runtime/src/tools/
  -> exec.rs: exec_command, kubectl_*
  -> files.rs: read_file, write_file, list_files
  -> patch.rs: apply_patch (unified diff parser)
  -> search.rs: search, grep_search, file_search
```

### Rust Crate Map

| Crate | Responsibility | Tests |
|-------|---------------|-------|
| `crates/cli` | Binary entry, TUI event loop, config, /memory /mcp /skills /hooks commands | — |
| `crates/tui` | App state, cells, chat, rendering, pager | 78 |
| `crates/runtime` | Agent loop, tool dispatch, compaction, hooks, skills | 81 |
| `crates/tools` | Tool schemas, classification, JSON Schema gen (spawn_agent added) | 13 |
| `crates/providers` | Anthropic provider, SSE streaming, tool calling | 14 |
| `crates/policy` | Policy modes, path rules, approval decisions | 57 |
| `crates/session-store` | JSONL recording, load/resume, index, per-project memory | 14 |
| `crates/mcp` | MCP stdio JSON-RPC 2.0 client (v2024-11-05 spec) | 5 |
| `crates/protocol` | Shared ServerMessage/ClientMessage types | — |
| `crates/mock-provider` | Deterministic test scenarios | — |
| `crates/markdown` | Markdown-to-ANSI renderer | — |
| `crates/clipboard` | Clipboard image paste (arboard) | — |

### Communication Model

In-process channels (no subprocess, no JSON-over-stdio):

```
CLI main task                          Agent task (tokio::spawn)
     │                                      │
     │  event_tx ──────────────────────►    │  AgentLoop::run()
     │  (mpsc::Sender<ServerMessage>)       │
     │                                      │
     │  ◄────────────────────── event_rx    │  emits Banner, AgentStatus,
     │  (mpsc::Receiver<ServerMessage>)     │  ToolStart, ToolResult, etc.
     │                                      │
     │  approval_tx ───────────────────►    │
     │  (mpsc::Sender<bool>)                │  waits on approval_rx
     │                                      │
```

### Protocol Surface

`crates/protocol/src/lib.rs` defines all message types.

**Client → Runtime:**
- `UserInput`, `SlashCommand`, `ApprovalResponse`, `Interrupt`

**Runtime → TUI:**
- `Banner`, `Mission`, `Clarification`, `ApprovalRequest`
- `ToolStart`, `ToolResult`, `FilesModified`
- `AgentMessage`, `AgentMessageDelta`, `ExecOutputDelta`
- `AgentStatus`, `TokenUsage`, `Spinner`
- `MissionComplete`, `Error`
- `ToolCall`, `ToolCallResult` (native function calling)
- `CommandList`
- `SubagentStart { task, depth }`, `SubagentComplete { task, outcome, summary, depth }` (subagent events)

### TUI Structure

```
crates/tui/

app.rs
  -> AgentPhase enum (10 phases: Idle, Planning, Thinking, Executing,
     ApplyingEdits, RunningChecks, Searching, Verifying, WaitingForApproval, Reading)
  -> InputMode: Normal, CommandPopup, ApprovalPending, HistorySearch, Pager, FileSearch

cell.rs
  -> Cell trait + 12 cell types:
     UserCell, MissionCell, ExecCell, AgentMessageCell, ErrorCell,
     ResultCell, ClarificationCell, ApprovalCell, TaskCard, DiffCard,
     GroupDivider, (future: specialized Docker/K8s/CI cards)
  -> is_read_only_tool() classification
  -> blink_on() animation helper

chat.rs
  -> ChatWidget: cell lifecycle, streaming, flush_and_emit()
  -> handle_server_message(): creates cells from ServerMessage
  -> TaskCard coalescing for read-only groups

bottom_pane.rs
  -> Live activity bar (braille 3x3 spinner, phase label, shimmer)
  -> User panel (input, ❯ prompt)
  -> Footer hints, context budget bar
  -> Command/file search popups, shortcuts overlay
```

### Session Persistence

```
~/.decipher/
  config.json                    <- api_key, model, base_url, policy_mode
  sessions/
    <uuid>.jsonl                 <- per-mission: meta header + events + session_end
    index.jsonl                  <- append-only index for fast listing
  memory/
    <16-hex-workspace-hash>/
      memories.jsonl             <- per-project persistent memory entries (id, ts, content)
  skills/
    <name>/SKILL.md              <- user-level skill fragments (YAML frontmatter)
  hooks.json                     <- shell lifecycle hooks (PreToolUse, PostToolUse, etc.)
  mcp.json                       <- MCP server configurations
```

JSONL format:
- Line 1: `session_meta` (thread_id, model, workspace, mission_goal, started_at)
- Lines 2..N: `event` (timestamp + ServerMessage payload)
- Last line: `session_end` (ended_at, outcome)

High-frequency events (AgentMessageDelta, ExecOutputDelta, Spinner, AgentStatus)
are filtered out — only meaningful protocol events are recorded.

### Extension Surface (post-R5)

```
Instructions (W1.1)
  ~/.decipher/DECIPHER.md  <- user-level (global defaults)
  <workspace>/DECIPHER.md  <- project-level (repo-specific)
  Both layers merged. Injected as "## Project Instructions" in system prompt.
  /init generates a starter template at workspace root.

Skills (P1.4)
  ~/.decipher/skills/<name>/SKILL.md  <- user-level
  {workspace}/.decipher/skills/<name>/SKILL.md  <- project-level (overrides user)
  Injected as "## Skills" block in system prompt.

Memory (P2.2)
  ~/.decipher/memory/<fnv1a-hash-of-workspace>/memories.jsonl
  Each entry: { id, ts, content }. Injected as "## Memory" block in system prompt.
  /memory list|add|clear slash commands.

Hooks (P1.2)
  ~/.decipher/hooks.json
  { "PreToolUse": [...], "PostToolUse": [...], "SessionStart": [...], "SessionEnd": [...] }
  Each hook: { command, match_tools? }. Subprocess receives JSON on stdin.
  Pre-hook blocking: non-zero exit code OR { "block": true } on stdout.

MCP (P2.1)
  ~/.decipher/mcp.json — list of { name, command, args, env } server configs
  Client uses stdio JSON-RPC 2.0. Tools injected into agent schema at session start.
  Unknown tool names route to matching MCP client in dispatch().
  /mcp slash command shows connected servers.

Subagents (P2.3)
  spawn_agent tool: { task, workspace?, max_turns? }
  MAX_DEPTH = 2 prevents runaway nesting.
  Uses same credentials; inherits no MCP clients.
  Events forwarded to parent TUI with [↓ Sub@N] prefix.
  AgentLoop::run is #[async_recursion] to handle the indirect recursive call chain.

Plan Mode (P1.6)
  decipher --plan "task"
  Agent runs with no tool schema (tools: None in API request) → text-only response.
  Emits MissionComplete { outcome: "PLAN" }.
  CLI captures plan text, shows approval prompt, then executes with full tools on yes.
```

## Implemented Features Summary

### Smart Card System (DONE)

`crates/runtime/src/output_parser.rs` parses exec_command output into 12 structured
types (TestSuite, DockerBuild, Compose, GitOp, Lint, KubePod, KubeLog, KubeEvent,
CI, EnvSetup, Migration + Generic fallback). The TUI renders specialized cards via
`render_smart_card_lines()` in `cell.rs`. 28 parser tests. AgentPhase has 18 context-aware
variants with `from_exec_cmd()` command detection.

### Parallel Tool Execution (DONE)

Read-only + Allow-policy tool calls run via `futures::future::join_all`. Write/exec/destructive
and any tool requiring approval run sequentially. Classification via `is_read_only_by_name()`.

### Extension Surface (DONE)

All four extension layers are implemented:
- **Hooks** (P1.2): `~/.decipher/hooks.json`, fire pre/post/session events
- **Skills** (P1.4): `~/.decipher/skills/` + `{workspace}/.decipher/skills/`, system prompt injection
- **Memory** (P2.2): per-workspace JSONL store, system prompt injection
- **MCP** (P2.1): `crates/mcp/` stdio JSON-RPC 2.0 client, tool injection

### Non-Interactive Mode (DONE)

`decipher exec "task"` with `--output-format text|json`, `--quiet`, exit codes 0-3.

### Plan Mode (DONE)

`decipher --plan "task"` runs agent in text-only mode for plan generation then prompts for approval.

### Subagents (DONE)

`spawn_agent` tool with depth limit, event forwarding, and `#[async_recursion]` for the recursive call chain.

### 5. Secondary Agent Migration (R5)

Remaining JS agents (planner, triage, fixer, verifier) should be ported to Rust
or deleted. `lib/api-client.js` is retained until this work completes.

## Codex-Derived Runtime Layers

The local `references/codex-main/codex-rs` tree shows a layered runtime.
DeCIpher has implemented the core layers (1-4, 6) and is working on the rest.

| Layer | Codex Reference | DeCIpher Status |
|-------|-----------------|-----------------|
| 1. Interaction Surface | tui, app-server-protocol | Done (P1.0) |
| 2. Thread/Turn Runtime | thread-store, rollout | Done (R3 session-store) |
| 3. Session Persistence | rollout, state | Done (R3 JSONL) |
| 4. Context Manager | core/compact.rs | Done (R2 compaction) |
| 5. Tool Orchestration | exec, tools | Done (R2 tools + Smart Card System) |
| 6. Approval/Policy | execpolicy | Done (R1 policy crate) |
| 7. Sandbox/Isolation | sandboxing | Not started |
| 8. Extension | hooks, instructions, core-skills, codex-mcp | Not started |
| 9. Collaboration | multi-agent, fork | Not started |
