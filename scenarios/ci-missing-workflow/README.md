# CI Missing Workflow

## Description

A Node.js project that has no GitHub Actions CI workflow. The `broken/` directory contains a
valid `package.json` and source file but no `.github/workflows/` directory.

DeCIpher's generation subsystem should detect the missing workflow and generate a minimal
Node.js CI workflow at `.github/workflows/ci.yml`.

## What is missing

`broken/` has no `.github/` directory at all. A CI workflow needs to be created from scratch.

## Expected result

After generation, the project should have `.github/workflows/ci.yml` that:
- triggers on push and pull_request events
- runs on ubuntu-latest
- checks out the code
- sets up Node.js 20
- runs `npm test`

## How DeCIpher handles this

1. Detects the `generate` mission type from the scenario's `mission_type` field
2. Inspects `broken/` to understand the project stack (Node.js via package.json)
3. Calls the generation subsystem with the mission goal
4. Writes `.github/workflows/ci.yml` to the target directory
