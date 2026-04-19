# Acceptance Criteria — docker-entrypoint-permission

## Reproduction
- Broken file: `scenarios/docker-entrypoint-permission/broken/Dockerfile`
- Log: `scenarios/docker-entrypoint-permission/logs/runtime-failure.log`

## Classification
- Expected: `permission_or_executable_error`

## Patch
- File: `Dockerfile`
- Add: `RUN chmod +x /entrypoint.sh` before the ENTRYPOINT line

## Verification Command (structural check)
```bash
grep -q 'chmod +x' scenarios/docker-entrypoint-permission/expected/Dockerfile && echo PASS || echo FAIL
```

## Pass Criteria
- Classification label matches `permission_or_executable_error`
- Patch adds chmod +x for the entrypoint script
- Structural verification passes (PASS)
