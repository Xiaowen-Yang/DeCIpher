# Acceptance Criteria — ci-python-version-drift

## Reproduction
- Broken file: `scenarios/ci-python-version-drift/broken/.github/workflows/ci.yml`
- Log: `scenarios/ci-python-version-drift/logs/ci-failure.log`

## Classification
- Expected: `dependency_version_mismatch`

## Patch
- File: `.github/workflows/ci.yml`
- Change: `python-version: '3.10'` → `python-version: '3.11'`

## Verification Command (structural check)
```bash
grep 'python-version' scenarios/ci-python-version-drift/expected/.github/workflows/ci.yml | grep -q '3.11' && echo PASS || echo FAIL
```

## Pass Criteria
- Classification label matches `dependency_version_mismatch`
- Patch changes python-version to 3.11
- Structural verification passes (PASS)
