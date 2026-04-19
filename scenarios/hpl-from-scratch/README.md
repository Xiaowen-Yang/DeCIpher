# hpl-from-scratch — Greenfield Scenario

## What This Tests

The user says one sentence:

> "I want to run an HPL benchmark on my macOS machine in a Docker container."

There are **no pre-existing files**. No Dockerfile, no scripts, no config.

DeCIpher must:

1. Understand that HPL needs MPI + BLAS/LAPACK + a runner script
2. Generate a Dockerfile from scratch
3. Generate a benchmark runner script
4. Build the Docker image (debug any build failures)
5. Start the container
6. Verify the benchmark can run

## How It Differs From Other Scenarios

| Aspect | Repair scenarios | This scenario |
|--------|-----------------|---------------|
| Start state | `broken/` directory with buggy files | Empty workspace |
| Expected state | `expected/` directory with correct files | None — acceptance checks only |
| Validation | File diff against expected/ | Runtime acceptance checks |
| Agent behavior | Fix known bugs | Research, generate, build, iterate |

## Acceptance Criteria

See `acceptance.json` — the agent passes when:
- A Dockerfile exists and contains HPL + MPI packages
- A run script exists
- `docker build` succeeds
- The container starts and can execute commands

## Running

```bash
node bin/decipher demo scenarios/hpl-from-scratch
```

This requires a configured API key — the agent must reason about
what to generate. There is no deterministic fallback.
