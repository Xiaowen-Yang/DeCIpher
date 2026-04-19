# Acceptance Criteria — docker-healthcheck-failure

## Reproduction

```bash
docker build -t demo-broken scenarios/docker-healthcheck-failure/broken
docker run -d --name hc-broken demo-broken
sleep 20
docker inspect hc-broken --format='{{.State.Health.Status}}'
# EXPECTED OUTPUT: unhealthy
docker rm -f hc-broken && docker rmi demo-broken
```

## Classification

- **Required label:** `healthcheck_startup_failure`
- **Confidence threshold:** >= 0.80

## Fix

The fix must change `8080` to `3000` in the HEALTHCHECK CMD line. No other lines should be modified.

## Verification

```bash
./scripts/verify.sh scenarios/docker-healthcheck-failure
```

Expected output: `PASS`

## Pass Criteria

- [ ] Triage classifies as `healthcheck_startup_failure`
- [ ] Confidence >= 0.80
- [ ] Patch targets only the HEALTHCHECK line
- [ ] `docker build` on expected succeeds
- [ ] Expected container reaches `healthy` via Docker healthcheck polling
