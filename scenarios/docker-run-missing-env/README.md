# Scenario: docker-run-missing-env

**Classification:** `missing_env_or_secret_contract`
**Difficulty:** Easy
**Docker verified:** Yes (real `docker build` + `docker run` ENV check)

## Problem

A Node.js API server Docker image builds successfully but the container exits immediately at runtime with:
```
FATAL: DATABASE_URL environment variable is required but not set
```

The application enforces a required environment variable at startup. The Dockerfile does not define a default value for `DATABASE_URL`, so the container fails unless the operator remembers to pass `-e DATABASE_URL=...` every time.

## Root Cause

The application validates `DATABASE_URL` at startup and calls `process.exit(1)` if it is missing. The Dockerfile is missing an `ENV DATABASE_URL=<default>` directive:

```dockerfile
# Missing ENV directive — container always fails without -e flag
CMD ["node", "server.js"]
```

## Fix

Add an `ENV` default in the Dockerfile so the container starts without requiring manual injection:

```diff
+ENV DATABASE_URL=sqlite:///app/data.db
 CMD ["node", "server.js"]
```

For production, the default can be overridden with `-e DATABASE_URL=postgres://...` or via compose `environment:`.

## Verification

Real `docker build` + `docker run` test:

```bash
# Build expected image
docker build -t decipher-env-check scenarios/docker-run-missing-env/expected

# Run and verify ENV is available
docker run --rm decipher-env-check node -e \
  "const v=process.env.DATABASE_URL; if(!v) process.exit(1); console.log('ENV OK:', v);"
# Output: ENV OK: sqlite:///app/data.db

# Confirm broken image fails at runtime
docker build -t test-broken scenarios/docker-run-missing-env/broken
docker run --rm test-broken
# FATAL: DATABASE_URL environment variable is required but not set
# exit code: 1

docker rmi decipher-env-check test-broken
```

## Demo Notes

- Build-time vs runtime failure distinction — very teachable moment
- Show `docker run` failing, then show fix, rebuild, `docker run` succeeding
- ENV contract is a common issue in team handoffs — relatable for judges
