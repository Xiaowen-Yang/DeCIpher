.PHONY: demo test doctor verify verify-agent install smoke

install:
	pnpm install

test:
	pnpm test

doctor:
	node bin/decipher doctor

demo:
	@echo "=== [1/6] Doctor check ==="
	node bin/decipher doctor
	@echo ""
	@echo "=== [2/6] Scenario: docker-copy-path-bug ==="
	node bin/decipher demo scenarios/docker-copy-path-bug
	@echo ""
	@echo "=== [3/6] Scenario: ci-python-version-drift ==="
	node bin/decipher demo scenarios/ci-python-version-drift
	@echo ""
	@echo "=== [4/6] Scenario: docker-healthcheck-failure ==="
	node bin/decipher demo scenarios/docker-healthcheck-failure
	@echo ""
	@echo "=== [5/6] Scenario: docker-multistage-wrong-artifact ==="
	node bin/decipher demo scenarios/docker-multistage-wrong-artifact
	@echo ""
	@echo "=== [6/6] Scenario: docker-run-missing-env ==="
	node bin/decipher demo scenarios/docker-run-missing-env
	@echo ""
	@echo "=== Demo sanity check complete ==="

# Structural verification: validates metadata verification_command only.
# Does NOT exercise the agent pipeline (triage/fix/verify loop).
# Runs without an API key.
verify:
	./scripts/verify.sh scenarios/docker-copy-path-bug
	./scripts/verify.sh scenarios/ci-python-version-drift
	./scripts/verify.sh scenarios/docker-healthcheck-failure
	./scripts/verify.sh scenarios/docker-multistage-wrong-artifact
	./scripts/verify.sh scenarios/docker-run-missing-env

# Agent-flow verification: runs the full demo pipeline for each scenario.
# Checks classification match AND verifier PASS. Requires a configured API key.
verify-agent:
	./scripts/verify.sh scenarios/docker-copy-path-bug --agent
	./scripts/verify.sh scenarios/ci-python-version-drift --agent
	./scripts/verify.sh scenarios/docker-healthcheck-failure --agent
	./scripts/verify.sh scenarios/docker-multistage-wrong-artifact --agent
	./scripts/verify.sh scenarios/docker-run-missing-env --agent

smoke:
	node bin/decipher demo scenarios/docker-copy-path-bug 2>&1 | grep -E 'Result:|Classification:' && echo "Smoke OK"
