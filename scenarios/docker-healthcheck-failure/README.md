# Scenario: docker-healthcheck-failure

**Classification:** `healthcheck_startup_failure`
**Difficulty:** Easy
**Docker verified:** Yes (real `docker build` in verification)

## Problem

A Node.js container starts and the server runs correctly on port 3000. However, `docker ps` always shows the container as `(unhealthy)`. Kubernetes/ECS/Swarm health gates block deployment because the container never passes its healthcheck.

## Root Cause

The `HEALTHCHECK` directive in the Dockerfile probes `http://localhost:8080/healthz` but the server listens on port **3000**. Every health probe times out, driving the container into `unhealthy` state.

```dockerfile
# BROKEN — wrong port
HEALTHCHECK CMD wget -qO- http://localhost:8080/healthz || exit 1
```

## Fix

Change the healthcheck port to match the server port:

```diff
-HEALTHCHECK --interval=5s --timeout=3s --retries=2 \
-  CMD wget -qO- http://localhost:8080/healthz || exit 1
+HEALTHCHECK --interval=5s --timeout=3s --retries=2 \
+  CMD wget -qO- http://localhost:3000/healthz || exit 1
```

## Verification

```bash
# Build expected image — must succeed
docker build -t decipher-hc-check scenarios/docker-healthcheck-failure/expected

# Confirm healthcheck uses correct port
grep 'localhost:3000' scenarios/docker-healthcheck-failure/expected/Dockerfile

# Optional: run container and check health
docker run -d --name hc-test decipher-hc-check
sleep 15
docker inspect hc-test --format='{{.State.Health.Status}}'
# Expected: healthy
docker rm -f hc-test
docker rmi decipher-hc-check
```

## Demo Notes

- Build succeeds for both broken and expected (misconfiguration is runtime, not build-time)
- Use `docker inspect` to show Health log entries with `ExitCode: 1`
- Contrast with expected where `ExitCode: 0` in health log
