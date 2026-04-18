# Acceptance Criteria — docker-copy-path-bug

## Reproduction
- Broken file: `scenarios/docker-copy-path-bug/broken/Dockerfile`
- Log: `scenarios/docker-copy-path-bug/logs/build-failure.log`

## Classification
- Expected: `path_or_copy_error`

## Patch
- File: `Dockerfile`
- Change: `COPY src/ .` → `COPY . .`

## Verification Command
```bash
docker build -t decipher-test scenarios/docker-copy-path-bug/expected && echo PASS || echo FAIL
```

## Pass Criteria
- Classification label matches `path_or_copy_error`
- Diff shows the COPY line change
- Verification exits 0 (PASS)
