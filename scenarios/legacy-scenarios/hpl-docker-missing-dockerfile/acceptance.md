# Acceptance Criteria

## Pass conditions

- [ ] `docker build scenarios/hpl-docker-missing-dockerfile/expected/ -q` exits 0
- [ ] `docker build scenarios/hpl-docker-missing-dockerfile/broken/ -q` exits non-zero
- [ ] The repaired Dockerfile uses `hpcc` instead of `hpl-benchmark-pkg`
- [ ] The repaired Dockerfile does not reference any non-existent apt packages

## Fail conditions

- [ ] The repaired Dockerfile still references `hpl-benchmark-pkg`
- [ ] The expected/ build fails to complete
