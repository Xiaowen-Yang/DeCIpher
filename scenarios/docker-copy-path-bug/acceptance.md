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
docker build -f scenarios/docker-copy-path-bug/expected/Dockerfile -t decipher-test . && echo PASS || echo FAIL
```

## Pass Criteria
- Classification label matches `path_or_copy_error`
- Diff shows the COPY line change
- Verification exits 0 (PASS)
