# Scenario: docker-multistage-wrong-artifact

**Classification:** `path_or_copy_error`
**Difficulty:** Medium
**Docker verified:** Yes (real `docker build` — broken FAILS, expected PASSES)

## Problem

A multi-stage Docker build has two stages: `builder` and `runner`. The builder stage compiles/prepares the application and writes output to `/src/output/`. The runner stage attempts to copy the artifact from `/src/dist/` — a path that was never created. The build fails at the `COPY --from=builder` instruction.

## Root Cause

The build script in the `builder` stage writes to `/src/output/app.js`:
```
RUN mkdir -p output && cp index.js output/app.js
```

But the `runner` stage references the wrong output directory:
```dockerfile
COPY --from=builder /src/dist/app.js ./app.js
#                         ^^^^ should be output/
```

This is a classic multi-stage copy path mismatch — the builder produces artifacts in one path, and the runner reads from a different path.

## Fix

```diff
-COPY --from=builder /src/dist/app.js ./app.js
+COPY --from=builder /src/output/app.js ./app.js
```

## Verification

Real Docker build test — broken image FAILS to build, expected image PASSES:

```bash
# Broken build — should FAIL
docker build -t test-broken scenarios/docker-multistage-wrong-artifact/broken
# ERROR: failed to solve: ... /src/dist/app.js: no such file or directory

# Expected build — should PASS
docker build -t test-expected scenarios/docker-multistage-wrong-artifact/expected
# Successfully tagged test-expected

docker run --rm test-expected
# app running

docker rmi test-broken test-expected
```

## Demo Notes

- This is the only scenario where the **broken Dockerfile actually fails to build** — very visual for demo
- Show the build error log, identify the COPY line, show the fix, rebuild successfully
