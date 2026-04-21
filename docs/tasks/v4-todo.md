# V4 — Active Backlog

Full archive: `docs/legacy/v4/TODO-ARCHIVE.md`
UI spec: `docs/v4/UI.md`
Test baseline: 291 Rust + 81 JS = 372 total (2026-04-21)

---

## Completed (this session)

| Item | Track | Status |
|------|-------|--------|
| Error visibility — 5 spawn sites emit ServerMessage::Error | Runtime | done |
| OpenAI-compatible provider (crates/providers/src/openai.rs) | Runtime | done, 10 tests |
| Provider auto-detection from config (anthropic/openai/auto) | Runtime | done |
| Lazy MCP initialization (startup → first mission) | Perf | done |
| Resume path loads hooks/skills/memory/MCP | Runtime | done |
| Activity bar: blinking ● dot (Claude Code style) | UI | done |
| Viewport shrunk 25→6 rows (banner stays visible) | UI | done |
| Duplicate banner on mission start removed | UI | done |
| Single tool card (no ● then ✓ — just ✓ on completion) | UI | done |
| Write/patch shows diff preview (+/- colored lines) | UI | done |
| Turn limit 20→200, display `[turn N]` (no max) | Runtime | done |
| Resize ghost lines fixed (terminal.clear on resize) | UI | done |

---

## Active — P1 (UX critical)

| Item | Track | Notes |
|------|-------|-------|
| P1.1 Streaming text visibility | UI | **done** — AgentMessageDelta streams text live to scrollback. Partial line shows in viewport with blinking cursor. AgentMessageDelta now sets agent_busy + spinner. |
| P1.2 Exec command live output | UI | **done** — exec_shell refactored to read stdout/stderr line-by-line via BufReader, streaming each line to TUI via ExecOutputDelta. Activity bar detail shows latest output line. |
| P1.3 Approval card in viewport | UI | **done** — Approval action detail (tool + reasoning) shown in viewport below activity bar with yellow bold styling. Cleared on approve/deny. |

## Active — P2 (quality)

| Item | Track | Notes |
|------|-------|-------|
| P2.1 Update UI.md spec to match reality | Docs | **done** — glyphs, motion, activity bar format all updated. |
| P2.2 Token/cost counter in activity bar | UI | **done** — Cumulative tokens shown in activity bar (e.g. `12.3K tok`). Fixed accumulation bug. |
| P2.3 Agent reasoning visible during generation | UI | **done** — streaming preview in viewport shows partial line + cursor. Completed lines go to scrollback. |
| P2.4 Exec output preview for success | UI | **done** — Successful exec shows last 3 lines of output (dim) + total line count. |

## Completed — P3

| Item | Track | Notes |
|------|-------|-------|
| P3.1 Diff viewer | UI | **done** — Write diff flows through output_preview → ExecCell pager transcript. DiffCard pager shows all files uncapped. Interactive hunk navigation is future. |
| P3.2 Multi-model switching | Runtime | **done** — `/model <name>` switches model + rebuilds provider + updates banner + terminal title. `/model` shows current. |
| P3.3 Session export (markdown) | Runtime | **done** — `/export [path]` exports transcript to .md via ChatWidget::transcript_lines. Auto-generates timestamped filename. |
| P3.4 MCP graceful shutdown | Runtime | **done** — Calls McpClient::shutdown() on all MCP clients before exit. |
| P3.5 shell_words quote handling | Runtime | **done** — Proper single/double quote and backslash-escape parsing. 3 new tests. |

## Wave 1 — Complete partial work + low-hanging fruit

| Item | Gap | Track | Notes |
|------|-----|-------|-------|
| W1.1 Instruction file loader (DECIPHER.md) | G14 | Runtime | **done** — `crates/runtime/src/instructions.rs` (10 tests). Two-layer `DECIPHER.md` loader, `format_instructions_section`, `generate_template`. Wired into AgentConfig, system prompt (after Environment, before Memory), banner display, `/init` command. |
| W1.2 Multi-model quirk table | G22 | Runtime | Reasoning-model detection (`is_reasoning_model()`). `max_completion_tokens` vs `max_tokens` translation. Thinking-mode parameter handling for GLM/o1/o3. Model compat matrix in provider. |
| W1.3 Git context injection | G18 | Runtime | At session start: inject branch name, HEAD sha, dirty-file count into system prompt. Single `git status --porcelain` + `git rev-parse` call. Staleness note on `/resume`. |

## Wave 2 — Cost reduction + test safety net

| Item | Gap | Track | Notes |
|------|-----|-------|-------|
| W2.1 Prompt cache optimization | G3 | Runtime | Anthropic `cache_control: {type: "ephemeral"}` on system prompt blocks. Estimate session tokens before compaction. `CompactionConfig` with tunable thresholds. System prompt reinjection after compaction. |
| W2.2 Reactive compaction + notification | G3 | UI/Runtime | Pressure-based trigger (not fixed threshold). Cache-break detection. User-visible compaction notification (P4.3). |
| W2.3 Mock provider + protocol tests | G10 | Test | `crates/mock-provider/` deterministic scenario server. Protocol-level integration tests (input → agent loop → ServerMessage sequence). TUI rendering snapshot tests (insta). |

## Wave 3 — Security hardening

| Item | Gap | Track | Notes |
|------|-----|-------|-------|
| W3.1 Bash argument validation | G2 | Runtime | Multi-layer validation: sed-arg check, destructive-pattern warning, read-only validation, command semantics analysis. Reference: Claude Code `bash_validation.rs` (9 layers, 1000+ LOC). |
| W3.2 Symlink escape + file safety | G2 | Runtime | `canonicalize()` before all file ops. Binary file detection (NUL-byte scan). Max read/write size limits. Workspace boundary enforcement. |
| W3.3 Per-tool approval policies | G9 | Policy | Per-tool rules (e.g. always-approve `read_file`, always-ask `exec_command`). Per-path overrides at tool level. Policy stacking for subagents. |

## Wave 4 — Power user features

| Item | Gap | Track | Notes |
|------|-----|-------|-------|
| W4.1 Session fork | G6 | Runtime | `fork` from any point in JSONL history → new thread with `ForkSnapshot`. `/fork` slash command. |
| W4.2 Session rollback | G6 | Runtime | Drop last N turns, persist rollback marker. `/rollback [N]` slash command. |
| W4.3 MCP server mode | G7 | Runtime | Expose DeCIpher as MCP server over stdio (`decipher mcp-server` subcommand). JSON-RPC 2.0 server. `ToolCallHandler` trait. |

## Wave 5 — Polish

| Item | Gap | Track | Notes |
|------|-----|-------|-------|
| W5.1 Web search tool | G17 | Runtime | Built-in `web_search` + `web_fetch` tools. Live vs cached modes. |
| W5.2 Shell completions | G19 | CLI | bash/zsh/fish completions via clap `generate`. REPL tab completion for slash commands. |
| W5.3 Interactive hunk-level diff navigation | — | UI | diff_render.rs exists, needs pager integration with j/k hunk navigation. |
| W5.4 /config command | — | Runtime | View/edit ~/.decipher/config.json from TUI. |
