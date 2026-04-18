# Acceptance Criteria — docker-run-missing-env

## Reproduction

```bash
docker build -t test-broken scenarios/docker-run-missing-env/broken
docker run --rm test-broken
# EXPECTED: exits with "FATAL: DATABASE_URL environment variable is required"
# EXPECTED exit code: 1
```

## Classification

- **Required label:** `missing_env_or_secret_contract`
- **Confidence threshold:** >= 0.85
- **Evidence:** must reference missing `DATABASE_URL` as the root cause

## Fix

Patch must add `ENV DATABASE_URL=sqlite:///app/data.db` (or equivalent default) to the Dockerfile. Only the Dockerfile needs modification.

## Verification

Real Docker build + run test:

```bash
docker build -t decipher-env-check scenarios/docker-run-missing-env/expected
docker run --rm decipher-env-check node -e \
  "const v=process.env.DATABASE_URL; if(!v) process.exit(1); console.log('ENV OK:', v);"
# EXPECTED: ENV OK: sqlite:///app/data.db
docker rmi decipher-env-check
```

## Pass Criteria

- [ ] Triage classifies as `missing_env_or_secret_contract`
- [ ] Confidence >= 0.85
- [ ] Evidence identifies `DATABASE_URL` as the missing variable
- [ ] Patch adds `ENV DATABASE_URL=...` to Dockerfile
- [ ] `docker build` on expected succeeds
- [ ] `docker run --rm` on expected shows `ENV OK: sqlite:///app/data.db`
