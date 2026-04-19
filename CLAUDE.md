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
When you change `agents/`, `prompts/`, `scenarios/`, `bin/`, or `docs/`, you are authoring the product.

## Commands

Primary development commands:

```bash
# Run the CLI
./bin/decipher

# Unit tests
pnpm test

# Demo scenarios
./bin/decipher demo scenarios/docker-copy-path-bug
./bin/decipher demo scenarios/ci-python-version-drift

# Environment check
./bin/decipher doctor

# Structural verification
make verify
```

## Source Of Truth

V2 documents are the only active product definition:
- `docs/v2/PRODUCT.md`
- `docs/v2/ARCHITECTURE.md`
- `docs/v2/COMMANDS.md`
- `docs/v2/SCENARIOS.md`
- `docs/v2/MEMORY.md`
- `docs/tasks/v2-todo.md`

Legacy material lives under `docs/legacy/` and is reference-only.

Implementation reference:
- `references/codex-main/`

Use Codex logic for session flow, resume, slash commands, approval handling,
history, and artifact visibility where it fits the V2 product.

## Architecture

V2 centers the product around:

`mission -> plan -> execute -> verify -> adapt -> complete`

The old `triage -> fix -> verify` flow remains as a repair subsystem.

Primary runtime shape:

```text
User Goal
   -> CLI Surface
   -> Mission Planner
   -> Session Memory + Clarification Gate
   -> Execution Loop
   -> Command Runner / Generation Subsystem / Repair Subsystem
   -> Verification Layer
   -> Review / Resume / Artifacts
   -> Mission Complete
```

## Repository Map

```text
bin/decipher               — CLI entry point and interactive surface
agents/executor/           — target resolution, approval, execution dispatch, resume
agents/planner/            — V2 mission planner layer
agents/triage/             — repair subsystem classifier
agents/fixer/              — repair subsystem patch generator
agents/verifier/           — verification and patch application utilities
lib/api-client.js          — OpenAI / Anthropic / custom provider abstraction
lib/cli-surface.js         — slash command definitions, view builders
lib/config.js              — ~/.decipher/config.json read/write
lib/history.js             — prompt history and reverse-search support
lib/mission.js             — V2 mission parsing and update logic
lib/mission-analyzer.js    — mission intent analysis
lib/mission-memory.js      — mission context persistence
lib/notifications.js       — terminal notification utilities
lib/popup.js               — popup/modal rendering
lib/preference-memory.js   — user preference persistence
lib/reporter.js            — structured output formatter
lib/session-store.js       — persisted mission/session state
lib/spinner.js             — terminal spinner animation
lib/template.js            — {variable} interpolation for prompt templates
prompts/                   — prompt contracts
scenarios/                 — deterministic fixtures and proving missions
docs/v2/                   — active V2 design docs
docs/legacy/               — archived V1/V1.5 material
docs/tasks/v2-todo.md      — active implementation backlog
docs/tasks/lessons.md      — reusable lessons from corrections
references/codex-main/     — implementation reference
```

## Workflow Orchestration

### Plan Discipline

- Enter explicit planning for any non-trivial task.
- If execution goes sideways, stop and re-plan instead of forcing the old path.
- Verification is part of the plan, not an afterthought.
- Prefer detailed, checkable specs before larger changes.

### Execution Discipline

- Work from `docs/tasks/v2-todo.md`.
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
- When relevant, compare intended V2 behavior against the legacy path being replaced.
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
  - `docs/tasks/v2-todo.md`
- Lessons learned:
  - `docs/tasks/lessons.md`

Rules:
- After any correction from the user, update `docs/tasks/lessons.md`.
- Capture the reusable mistake pattern and the rule that should prevent it.
- Keep progress logs current in `docs/tasks/v2-todo.md`.

## Git Rules

- Local commits: allowed (`feat:`, `fix:`, `chore:` format)
- `git push`: never without explicit user instruction
- Force push or `--no-verify`: never
