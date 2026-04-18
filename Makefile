.PHONY: demo test doctor verify install

install:
	pnpm install

test:
	pnpm test

doctor:
	node bin/decipher doctor

demo:
	@echo "=== [1/3] Doctor check ==="
	node bin/decipher doctor
	@echo ""
	@echo "=== [2/3] Scenario: docker-copy-path-bug ==="
	node bin/decipher demo scenarios/docker-copy-path-bug
	@echo ""
	@echo "=== [3/3] Scenario: ci-python-version-drift ==="
	node bin/decipher demo scenarios/ci-python-version-drift
	@echo ""
	@echo "=== Demo sanity check complete ==="

verify:
	./scripts/verify.sh scenarios/docker-copy-path-bug
	./scripts/verify.sh scenarios/ci-python-version-drift

smoke:
	node bin/decipher demo scenarios/docker-copy-path-bug 2>&1 | grep -E '^\[|✓|✗' && echo "Smoke OK"
