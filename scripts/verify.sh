#!/usr/bin/env bash
set -euo pipefail

SCENARIO="${1:-}"
if [ -z "$SCENARIO" ]; then
  echo "Usage: ./scripts/verify.sh <scenario-path>"
  exit 1
fi

if [ ! -d "$SCENARIO" ]; then
  echo "Error: scenario directory not found: $SCENARIO"
  exit 1
fi

echo "=== Verifying scenario: $SCENARIO ==="

# Check metadata
METADATA="$SCENARIO/metadata.json"
if [ ! -f "$METADATA" ]; then
  echo "Error: missing metadata.json in $SCENARIO"
  exit 1
fi

EXPECTED_CLASS=$(node -e "const m=JSON.parse(require('fs').readFileSync('$METADATA','utf8')); console.log(m.expected_classification)")
VERIFY_CMD=$(node -e "const m=JSON.parse(require('fs').readFileSync('$METADATA','utf8')); console.log(m.verification_command)")

echo "Expected classification: $EXPECTED_CLASS"
echo "Verification command:    $VERIFY_CMD"
echo ""

# Run verification command
echo "Running verification..."
if eval "$VERIFY_CMD"; then
  echo ""
  echo "=== PASS ==="
  exit 0
else
  echo ""
  echo "=== FAIL ==="
  exit 1
fi
