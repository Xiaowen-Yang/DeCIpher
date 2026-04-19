# Acceptance Criteria — docker-runtime-port-mismatch-loop

## Classification

- [ ] Agent classifies failure as `healthcheck_startup_failure`
- [ ] Confidence ≥ 0.85
- [ ] Evidence includes port mismatch (8080 vs 3000)

## Executor Loop (Phase 5.5 success criteria)

- [ ] `docker build broken/` completes with exit 0 (build succeeds)
- [ ] Container starts but becomes **unhealthy** within 30 seconds
- [ ] Agent captures the healthcheck failure output (wget connection refused)
- [ ] Agent produces a patch that changes port 8080 → 3000 in HEALTHCHECK
- [ ] Patch is applied in a temp workspace (original broken/ untouched during loop)
- [ ] Rebuild in temp workspace succeeds
- [ ] Re-run in temp workspace: container becomes **healthy**
- [ ] Repaired `Dockerfile` is written back to `scenarios/.../broken/`

## Output

- [ ] Output reads as an execution transcript (state transitions visible)
- [ ] No clarifying questions asked — loop started automatically
- [ ] Workspace path reported if loop fails (for debugging)

## Verification Command

```bash
./scripts/verify.sh scenarios/docker-runtime-port-mismatch-loop
```

Expected: PASS (expected/Dockerfile builds + contains correct port)
