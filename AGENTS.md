# Repository Guidelines

## Project Structure & Module Organization

`bin/decipher` is the CLI entry point. Core runtime code lives in `agents/` (`executor/`, `orchestrator/`, `triage/`, `fixer/`, `verifier/`) and shared utilities live in `lib/`. Prompt templates belong in `prompts/`, shell helpers in `scripts/`, and domain knowledge in `skills/`. Demo fixtures live under `scenarios/<name>/` with `broken/`, `expected/`, `logs/`, and `metadata.json`. Unit tests live in `tests/unit/`. Reference material in `references/` and `decipher-agent-docs/` is read-only unless the task explicitly targets it.

## Build, Test, and Development Commands

- `pnpm install`: install dependencies.
- `pnpm test` or `make test`: run the `node:test` unit suite.
- `make doctor`: run environment checks through `node bin/decipher doctor`.
- `make verify`: validate scenario metadata and canned verification commands without an API key.
- `make verify-agent`: run the full triage-fix-verify flow for scenarios; requires configured model credentials.
- `node bin/decipher demo scenarios/docker-copy-path-bug`: exercise one scenario locally.

## Coding Style & Naming Conventions

Use Node.js ESM with explicit imports and small focused modules. Existing runtime code uses double quotes, trailing commas, and semicolons; shell scripts use `bash` with `set -euo pipefail`. Keep directory names lowercase and hyphenated for scenarios (`docker-copy-path-bug`), and name tests `*.test.js`. Prefer minimal patches that keep agent boundaries clear.

## Testing Guidelines

Add or update unit tests in `tests/unit/` for parser, reporter, and verifier behavior changes. Prefer deterministic tests over live API calls. For scenario changes, run `make verify`; use `make verify-agent` only when the end-to-end agent loop is relevant and credentials are available.

## Commit & Pull Request Guidelines

Git history mostly follows concise Conventional Commit prefixes such as `feat:`, `fix:`, and `chore:`; keep using them. PRs should explain the failure mode, the minimal fix, and the verification command you ran. Include terminal output or screenshots only when they clarify CLI behavior.
