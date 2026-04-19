# Acceptance Criteria — docker-multistage-wrong-artifact

## Reproduction

```bash
docker build -t test-broken scenarios/docker-multistage-wrong-artifact/broken
# EXPECTED: build FAILS with "COPY --from=builder /src/dist/app.js: not found"
```

## Classification

- **Required label:** `path_or_copy_error`
- **Confidence threshold:** >= 0.85
- **Evidence:** error message must reference the incorrect source path `/src/dist/`

## Fix

The patch must change `/src/dist/app.js` to `/src/output/app.js` in the COPY instruction. Only the Dockerfile needs modification.

## Verification

```bash
# Real Docker build — must succeed
docker build -t decipher-ms-check scenarios/docker-multistage-wrong-artifact/expected && echo PASS
docker run --rm decipher-ms-check
docker rmi decipher-ms-check
```

Expected output: `PASS` then `app running`

## Pass Criteria

- [ ] Triage classifies as `path_or_copy_error`
- [ ] Confidence >= 0.85
- [ ] Evidence references `/src/dist/` as the wrong path
- [ ] Patch changes COPY source to `/src/output/app.js`
- [ ] `docker build` on expected succeeds (real build verification)
- [ ] `docker run --rm` on expected outputs `app running`
