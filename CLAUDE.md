# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

**DeCIpher** is a mission-driven local execution agent with a CI/deployment specialty.

It is not just a CI debugging assistant and it is not just a scenario runner.
It should feel closer to a Codex-style general agent that is strongest at:
- CI failure analysis
- Docker build/runtime repair
- deployment workflows
- environment setup
- mission-bounded execution loops

**You are the development agent building DeCIpher. You are not DeCIpher.**
When you change `agents/`, `prompts/`, `bin/`, or `docs/`, you are authoring the product.

## Commands

```bash
# Run the CLI
./bin/decipher

# Node.js unit tests
pnpm test

# Run a single Node.js test file
node --test tests/unit/<file>.test.js

# Environment check
./bin/decipher doctor

# Structural verification (no API key needed — validates scenario metadata only)
make verify

# Full agent-flow verification (requires configured API key)
make verify-agent

# Smoke test (quick pipeline sanity check)
make smoke

# Run all demo scenarios
make demo

# Rust: build all crates
cargo build

# Rust: run all crate tests
cargo test

# Rust: test a single crate (e.g. tui)
cargo test -p decipher-tui

# Rust: build the TUI binary
cargo build --bin decipher-tui
```

## Source Of Truth

V4 documents are the active product definition:
- `docs/v4/README.md`
- `docs/v4/DEVELOPMENT.md`
- `docs/v4/PRODUCT.md`
- `docs/v4/ARCHITECTURE.md`
- `docs/v4/COMMANDS.md`
- `docs/v4/GAPS.md`
- `docs/v4/UI.md`
- `docs/tasks/v4-todo.md`
- `docs/tasks/lessons.md`

Legacy material lives under `docs/legacy/` (v1, v2, v3, old specs, archived plans) and is reference-only.

Ownership rules:
- `docs/v4/README.md` defines the doc ownership matrix.
- `docs/v4/DEVELOPMENT.md` is the developer navigation layer only.
- `docs/tasks/v4-todo.md` is the only roadmap and priority owner.
- `docs/v4/GAPS.md` owns competitor comparison and gap evidence, not implementation order.
- `docs/v4/COMMANDS.md` must match actual command surface and current semantics.
- `docs/v4/UI.md` owns the visual language, card renderings, and interaction surface design.
- `docs/v4/PRODUCT.md` and `docs/v4/ARCHITECTURE.md` can describe direction, but should not restate backlog ordering tables.

Implementation reference:
- `references/codex-main/`
- `references/claude-code-main/`
- `references/free-code-main/`

Use Codex as the end-state Rust-native architecture reference.
Use Claude Code (`claude-code-main`) as the migration-process and parity-harness reference.
Use free-code as the feature-pattern and UX reference — it is the exposed TypeScript source of
Claude Code (Bun + React/Ink, 88 compile-time feature flags). Use it for feature design and
interaction ideas only; do not copy its JS/TS implementation into DeCIpher's Rust codebase.

Implementation language direction:
- Current reality (post-R4): Rust owns the full runtime path. Node.js is a thin shim.
- `bin/decipher` is a small Node.js entry point (~160 lines) that delegates to the Rust TUI binary for interactive use and handles non-interactive CLI subcommands.
- The JS fallback interactive mode was deleted in R4. There is no Node.js agent loop.
- New runtime capability goes into Rust crates only.
- Secondary JS agents deleted in R5. Only `bin/decipher` (thin shim) and `agents/executor/` remain.
- Runtime migration is complete (R0-R5 done). See archive: `docs/legacy/plans/2026-04-20-rust-native-runtime-program.md`.

## Documentation Lookup By Stage

When entering or changing the repo, use this order instead of guessing:

### 1. Orientation

Read:
- `docs/v4/README.md`
- `docs/v4/DEVELOPMENT.md`
- `docs/v4/PRODUCT.md`

Goal:
- understand what DeCIpher is trying to be
- understand which doc owns which truth
- avoid writing roadmap or product content into the wrong file
- understand that current runtime ownership is mixed, but target architecture is Rust-native and JS compatibility is not a constraint

### 2. Runtime And Architecture Work

Read:
- `docs/v4/DEVELOPMENT.md`
- `docs/v4/ARCHITECTURE.md`
- `docs/tasks/v4-todo.md`
- `references/codex-main/` for Rust-native end-state design
- `references/claude-code-main/` for migration sequencing and parity strategy

Use this stage for:
- native tool calling
- policy versus sandbox boundaries
- session/thread/turn model
- context compaction
- hooks, skills, instructions, MCP, subagents
- deciding how to move ownership into Rust rather than preserving the JS backend

### 3. Command And UX Semantics

Read:
- `docs/v4/COMMANDS.md`
- `docs/v4/PRODUCT.md`
- `docs/v4/UI.md` for interaction surface design, card types, visual language
- `docs/tasks/v4-todo.md` if the command is planned rather than implemented

Use this stage for:
- slash commands
- flags
- interactive versus non-interactive behavior
- keeping `/plan` and `/resume` semantics precise
- interaction-surface changes such as the live activity bar, task timeline,
  diff cards, detail surface, and result-card replacement

### 4. Gap Analysis And Parity Research

Read:
- `docs/v4/GAPS.md`
- `docs/v4/ARCHITECTURE.md`
- `references/codex-main/`

Use this stage for:
- understanding why a feature matters
- grounding design choices in Codex/Claude/Gemini patterns
- updating competitor evidence without changing backlog order

### 5. Implementation Planning

Read:
- `docs/tasks/v4-todo.md`
- `docs/v4/ARCHITECTURE.md`
- `docs/v4/UI.md` for TUI/interaction work — card types, visual language, phase states
- `docs/v4/COMMANDS.md` if user-visible semantics are involved

Rules:
- change priority only in `docs/tasks/v4-todo.md`
- do not create competing phase tables in V4 docs
- if a term drifts, fix the owning doc and then update navigation docs
- if TUI or transcript work is involved, refer to `docs/v4/UI.md` over ad hoc visual decisions

### 6. After Code Changes

Update only the owning docs:
- runtime or protocol boundary changed: `docs/v4/ARCHITECTURE.md`
- command or flag behavior changed: `docs/v4/COMMANDS.md`
- product positioning changed: `docs/v4/PRODUCT.md`
- rationale or competitor comparison changed: `docs/v4/GAPS.md`
- delivery order changed: `docs/tasks/v4-todo.md`
- developer lookup guidance changed: `docs/v4/README.md`, `docs/v4/DEVELOPMENT.md`, `CLAUDE.md`

Term discipline:
- `/resume [id]` loads a saved session from JSONL and continues with restored LLM context
- `/plan` currently means plan view only, not review-before-execute mode (P1.6)
- approval policy and sandbox transform are different layers
- session/thread, turn, and item are different runtime levels
- smart cards = structured output parsing for Docker/K8s/CI/test rendering (complete, see `docs/v4/UI.md`)

## Architecture

V4 runtime flow (post-R4, actual execution path):

```text
bin/decipher (thin JS shim, ~160 lines)
  → execs decipher-tui (Rust, crates/cli/)
    → AgentLoop::run() (in-process tokio task)
      → AnthropicProvider → streaming LLM call
        → tool dispatch (crates/tools/ + crates/runtime/)
          → approval policy (crates/policy/)
            → ServerMessage events → TUI via mpsc channel
              → session recorded to ~/.decipher/sessions/<uuid>.jsonl
```

Language ownership (post-R4):
- Rust owns everything: TUI, protocol, agent loop, tool dispatch, providers, policy, session store.
- Node.js: `bin/decipher` entry shim + secondary agents (triage, fixer, verifier, planner) pending R5.
- Deleted in R4: `lib/server-mode.js`, `lib/agent-loop.js`, `lib/mission-analyzer.js`,
  `lib/compact.js`, `lib/session-store.js`, `agents/executor/agent-loop.js`, `agents/executor/tools.js`,
  `crates/agent-bridge`.

Planning and resume terminology:
- `/plan` currently means plan view only. It shows the current mission plan; it is not a review-before-execute gate.
- `/resume [thread_id]` resumes a session from `~/.decipher/sessions/<uuid>.jsonl` by reconstructing
  LLM message history and feeding it to a fresh AgentLoop as `resume_from`.
- `/resume` with no arg picks the most recent session from the index.

## Repository Map

```text
bin/decipher               — CLI entry point (thin JS shim, ~160 lines)
bin/decipher-tui           — Rust TUI binary (built from crates/cli/)
crates/                    — Rust workspace (Cargo workspace at project root)
  cli/                     — binary entry point, event loop, key handling, /resume
  tui/                     — app state machine, terminal rendering (78 tests)
  protocol/                — shared JSON protocol types
  markdown/                — markdown-to-ANSI renderer
  clipboard/               — clipboard image paste (arboard)
  providers/               — LLM provider HTTP/streaming (Anthropic)
  policy/                  — approval policy and path-rule evaluation
  tools/                   — tool schema definitions and classification
  runtime/                 — AgentLoop, tool dispatch, context compaction (31 tests)
  session-store/           — append-only JSONL session history, resume loader (8 tests)
agents/executor/           — target resolution (R5 candidate for Rust migration)
agents/planner/            — mission planner layer (R5 candidate)
agents/triage/             — repair subsystem classifier (R5 candidate)
agents/fixer/              — repair subsystem patch generator (R5 candidate)
agents/verifier/           — verification and patch application utilities (R5 candidate)
lib/api-client.js          — OpenAI / Anthropic provider abstraction (used by secondary agents)
lib/cli-surface.js         — slash command definitions, view builders
lib/config.js              — ~/.decipher/config.json read/write
lib/mission-memory.js      — mission context persistence
lib/notifications.js       — terminal notification utilities
lib/reporter.js            — structured output formatter
lib/spinner.js             — terminal spinner animation (agent subprocess)
lib/template.js            — {variable} interpolation for prompt templates
prompts/                   — prompt contracts
scenarios/                 — deterministic fixtures and proving missions
scripts/bump-version.sh    — version bump, commit, and tag script
.github/workflows/ci.yml   — CI: test on push/PR (Node 22, 24)
.github/workflows/release.yml — Release: tag push → npm publish + GitHub Release
docs/v4/                   — active V4 product definition + gap analysis
docs/specs/                — design specs for in-progress features
docs/legacy/               — archived V1/V2/V3 material + completed work
docs/tasks/v4-todo.md      — active implementation backlog and only roadmap owner
docs/tasks/lessons.md      — reusable lessons from code review
references/codex-main/     — implementation reference
DECIPHER.md                — project instruction file (auto-loaded into system prompt; /init to scaffold)
```

## Workflow Orchestration

### Plan Discipline

- Enter explicit planning for any non-trivial task.
- If execution goes sideways, stop and re-plan instead of forcing the old path.
- Verification is part of the plan, not an afterthought.
- Prefer detailed, checkable specs before larger changes.

### Execution Discipline

- Work from `docs/tasks/v4-todo.md`.
- Do not create competing roadmap tables in `PRODUCT.md`, `ARCHITECTURE.md`, `COMMANDS.md`, or `GAPS.md`.
- If you change command semantics, update `docs/v4/COMMANDS.md`.
- If you change product direction or runtime boundaries, update the owning V4 doc instead of scattering notes.
- Prefer Rust for all new long-lived runtime ownership. Do not preserve JS compatibility as a product constraint.
- Move phase by phase, step by step.
- Implement one small batch at a time.
- Verify each batch before claiming progress.
- Mark completed items and progress notes as you go.
- Do not stop to ask the user for routine confirmation.
- Only ask when the requirement, boundary, or safety constraint is genuinely unclear.

### Subagent / Parallel Work

- Prefer focused parallel exploration when tasks are independent.
- Keep one clear responsibility per delegated unit of work.
- Use parallelism to keep the main context clean, not to duplicate work.

### Verification Before Completion

- Never mark work complete without proof.
- Run targeted tests first, then broader regression checks.
- When relevant, compare intended behavior against the legacy path being replaced.
- Hold the bar at "would a strong staff engineer approve this change?"

### Simplicity And Elegance

- Start with the simplest design that fully solves the problem.
- If a fix feels hacky, stop and implement the cleaner structure.
- Do not over-engineer small changes.
- Minimize surface area and preserve clean module boundaries.

### Autonomous Bug Fixing

- If given a bug or failing behavior, diagnose it and fix it directly.
- Use logs, failing tests, and runtime evidence.
- Do not ask the user to drive normal debugging steps.

## Task And Lessons Tracking

- Active backlog:
  - `docs/tasks/v4-todo.md`
- Lessons learned:
  - `docs/tasks/lessons.md`
- Industry gap reference:
  - `docs/v4/GAPS.md`

Rules:
- After any correction from the user, update `docs/tasks/lessons.md`.
- Capture the reusable mistake pattern and the rule that should prevent it.
- Keep progress logs current in `docs/tasks/v4-todo.md`.

## Git Rules

- Local commits: allowed (`feat:`, `fix:`, `chore:` format)
- `git push`: never without explicit user instruction
- Force push or `--no-verify`: never
