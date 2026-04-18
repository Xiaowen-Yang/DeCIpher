#!/usr/bin/env bash
set -euo pipefail

echo "=== [1/3] Doctor check ==="
node bin/decipher doctor

echo ""
echo "=== [2/3] Scenario: docker-copy-path-bug ==="
node bin/decipher demo scenarios/docker-copy-path-bug

echo ""
echo "=== [3/3] Scenario: ci-python-version-drift ==="
node bin/decipher demo scenarios/ci-python-version-drift

echo ""
echo "=== Demo sanity check complete ==="
