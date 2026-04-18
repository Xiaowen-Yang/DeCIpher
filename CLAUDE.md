# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

**DeCIpher** is a focused agent-first CLI (`decipher`) for automated CI/deployment troubleshooting. It is not a general coding assistant — it exclusively handles CI pipeline failures, Docker build errors, deployment issues, and environment setup problems.

**You are the development agent building DeCIpher. You are not DeCIpher.** When you write code in `agents/`, `prompts/`, or `scenarios/`, you are authoring the product — not executing its behaviors yourself. Never switch roles mid-task.

## Commands

The project is currently in the design/planning phase. Once implemented, the primary development commands will be:

```bash
# Run the CLI (once implemented)
node bin/decipher

# Demo scenarios
node bin/decipher demo scenarios/docker-copy-path-bug
node bin/decipher demo scenarios/ci-python-version-drift

# Environment check
node bin/decipher doctor

# Build / test / verify (make targets, once available)
make build
make test
make smoke
make doctor
make verify
```

## Architecture

DeCIpher is a structured pipeline with named nodes, not a monolithic agent. Each node produces a typed JSON artifact — this makes every stage independently testable and demo-visible.

```
bin/decipher (CLI entry)
    → Context Collector + Scenario Loader
    → Triage Node          → classification artifact
    → Fix Proposal Node    → patch artifact
    → Patch Executor
    → Verification Node    → pass/fail artifact (loops back to Fix, max 3 iterations)
    → Report Output
```

**Failure stop policy:** The orchestrator stops early and returns `NEEDS_HUMAN_REVIEW` if triage confidence < 0.7, the same patch is attempted twice, or the patch touches more than 2 files.

**Config:** `~/.decipher/config.json` — stores provider, model, API key, `max_iterations`, `auto_approve`. `auto_approve` defaults to `false`; DeCIpher always prompts before executing changes.

## Repository Map

```
bin/decipher           — CLI entry point (Node.js ESM, thin orchestration shell)
agents/orchestrator/   — task router and iteration loop controller
agents/triage/         — failure classifier (calls AI API with triage prompt)
agents/fixer/          — minimal fix proposal generator
agents/verifier/       — runs verification commands and captures evidence
prompts/               — Markdown prompt templates with {variable} placeholders
skills/                — domain knowledge files injected per node (not global)
scripts/               — pure shell commands (collect_context.sh, doctor.sh, verify.sh, demo.sh)
scenarios/             — deterministic demo fixtures (broken/, expected/, logs/, acceptance.md)
docs/                  — architecture docs, acceptance criteria, runbooks
tasks/                 — todo.md, lessons.md, review.md, last-known-good.md
references/codex-main/ — reference CLI for UX/workflow patterns (read-only reference)
decipher-agent-docs/   — full agent contract docs (AGENTS.md is source of truth)
```

## Key Documents

Read these before any significant work:

- `decipher-agent-docs/AGENTS.md` — source of truth for all operating rules
- `decipher-agent-docs/ARCHITECTURE.md` — detailed system design and module specs
- `decipher-agent-docs/COMMANDS.md` — full CLI command reference and output format
- `tasks/lessons.md` — mistakes to avoid, project constraints
- `tasks/todo.md` — current plan and progress

## Task Management

**Before coding:** Update `tasks/todo.md` with a checklist plan. Read `tasks/lessons.md`.

**After corrections:** Update `tasks/lessons.md` immediately with the mistake pattern as a reusable rule.

**Done criteria:** Issue evidenced → fix explicit → verification shown (command + output + conclusion) → result written to `tasks/review.md`.

## Failure Taxonomy

The 10 built-in classification labels used by the triage node:
`dependency_version_mismatch`, `missing_env_or_secret_contract`, `path_or_copy_error`, `permission_or_executable_error`, `docker_entrypoint_runtime_error`, `healthcheck_startup_failure`, `test_regression`, `ci_config_drift`, `cache_or_lockfile_issue`, `needs_more_evidence`

## Demo Priority

When scope must be cut, prioritize in order:
1. `scenarios/docker-copy-path-bug` — Docker COPY path error, auto-patchable
2. `scenarios/ci-python-version-drift` — CI workflow version mismatch
3. `scenarios/env-missing-node` — doctor/bootstrap flow
4. `scenarios/docker-entrypoint-permission` — permission error recovery

## Git Rules

- Local commits: allowed (`feat:`, `fix:`, `chore:` format)
- `git push`: never without explicit user instruction
- Force push or `--no-verify`: never
