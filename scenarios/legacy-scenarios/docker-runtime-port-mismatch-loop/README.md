# docker-runtime-port-mismatch-loop

**Category:** docker  
**Difficulty:** medium  
**Classification:** `healthcheck_startup_failure`  
**Execution mode:** `healthcheck` (build succeeds, runtime healthcheck fails)

---

## Broken State

The Dockerfile builds successfully. The Node.js HTTP server starts and listens
on **port 3000**. However the `HEALTHCHECK` instruction probes **port 8080**:

```dockerfile
HEALTHCHECK --interval=5s --timeout=3s --retries=3 \
  CMD wget -qO- http://localhost:8080/ || exit 1
```

Docker marks the container **unhealthy** after 3 retries — `wget` gets
"Connection refused" because nothing is listening on 8080.

---

## Fix

Change the HEALTHCHECK port from `8080` to `3000`:

```diff
-  CMD wget -qO- http://localhost:8080/ || exit 1
+  CMD wget -qO- http://localhost:3000/ || exit 1
```

---

## Executor Loop Behavior

This scenario is designed to exercise the full `healthcheck` execution mode:

1. `docker build broken/` → **succeeds** (no build-time error)
2. `docker run -d` → container starts
3. Poll `docker inspect .Health.Status` → **unhealthy** after 3 failed probes
4. Capture `docker logs` + healthcheck log output
5. Triage: `healthcheck_startup_failure` — port mismatch
6. Patch: correct HEALTHCHECK port in temp workspace
7. Rebuild → rerun → poll → **healthy**
8. Write-back repaired Dockerfile to `broken/`
