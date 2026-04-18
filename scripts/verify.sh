#!/usr/bin/env bash
# verify.sh — two modes:
#   structural (default): executes the canned verification_command from metadata.json
#   agent      (--agent): runs the full decipher demo pipeline, checks classification + PASS
set -euo pipefail

MODE="structural"
SCENARIO="${1:-}"

# Parse flags
for arg in "$@"; do
  case "$arg" in
    --agent) MODE="agent" ;;
    -*) echo "Unknown flag: $arg"; exit 1 ;;
    *)  SCENARIO="$arg" ;;
  esac
done

if [ -z "$SCENARIO" ]; then
  echo "Usage: ./scripts/verify.sh <scenario-path> [--agent]"
  echo ""
  echo "  structural (default): validate the verification_command from metadata.json"
  echo "  --agent:              run the full demo pipeline and check classification + PASS"
  exit 1
fi

if [ ! -d "$SCENARIO" ]; then
  echo "Error: scenario directory not found: $SCENARIO"
  exit 1
fi

METADATA="$SCENARIO/metadata.json"
if [ ! -f "$METADATA" ]; then
  echo "Error: missing metadata.json in $SCENARIO"
  exit 1
fi

EXPECTED_CLASS=$(node -e "const m=JSON.parse(require('fs').readFileSync('$METADATA','utf8')); console.log(m.expected_classification)")
VERIFY_CMD=$(node -e "const m=JSON.parse(require('fs').readFileSync('$METADATA','utf8')); console.log(m.verification_command)")

echo "=== Verifying scenario: $SCENARIO ==="
echo "Expected classification: $EXPECTED_CLASS"
echo "Mode:                    $MODE"
echo ""

if [ "$MODE" = "agent" ]; then
  # Full agent-flow check: run demo, extract classification and PASS/FAIL from output.
  # Temporarily disable set -e so a non-zero demo exit code doesn't abort the script
  # before we can parse and report the actual classification / result.
  echo "Running: node bin/decipher demo $SCENARIO"
  set +e
  DEMO_OUTPUT=$(node bin/decipher demo "$SCENARIO" 2>&1)
  DEMO_EXIT=$?
  set -e
  echo "$DEMO_OUTPUT"
  echo ""

  # Extract classification and result — use portable awk (no grep -P on macOS)
  ACTUAL_CLASS=$(echo "$DEMO_OUTPUT" | awk '/label:/{print $NF}' | head -1 || true)
  ACTUAL_RESULT=$(echo "$DEMO_OUTPUT" | awk '/Result:/{print $NF}' | head -1 || true)

  PASS=true
  if [ -z "$ACTUAL_CLASS" ]; then
    echo "ERROR: could not extract classification from demo output"
    PASS=false
  elif [ "$ACTUAL_CLASS" != "$EXPECTED_CLASS" ]; then
    echo "CLASSIFICATION MISMATCH: expected=$EXPECTED_CLASS actual=$ACTUAL_CLASS"
    PASS=false
  else
    echo "Classification: MATCH ($ACTUAL_CLASS)"
  fi

  # SKIPPED is accepted for auto_fixable:false scenarios (no patch, no env verification)
  if [ "$ACTUAL_RESULT" = "PASS" ] || [ "$ACTUAL_RESULT" = "SKIPPED" ]; then
    echo "Verification:   $ACTUAL_RESULT"
  else
    echo "Verification:   FAIL (result=${ACTUAL_RESULT:-none})"
    PASS=false
  fi

  echo ""
  if [ "$PASS" = "true" ]; then
    echo "=== PASS ==="
    exit 0
  else
    echo "=== FAIL ==="
    exit 1
  fi

else
  # Structural mode: validate the scenario's verification_command from metadata.json.
  # Exception: auto_fixable:false scenarios cannot be verified by running a command
  # on the developer's machine (e.g. `node --version` always passes). Instead, validate
  # that the required fixture files are present.
  AUTO_FIXABLE=$(node -e "const m=JSON.parse(require('fs').readFileSync('$METADATA','utf8')); console.log(m.auto_fixable !== false ? 'true' : 'false')")

  if [ "$AUTO_FIXABLE" = "false" ]; then
    echo "Scenario is not auto-fixable — validating fixture structure"
    MISSING=0
    for f in README.md acceptance.md logs; do
      if [ ! -e "$SCENARIO/$f" ]; then
        echo "  MISSING: $f"
        MISSING=1
      else
        echo "  OK:      $f"
      fi
    done
    echo ""
    if [ "$MISSING" = "0" ]; then
      echo "=== PASS (structural) ==="
      exit 0
    else
      echo "=== FAIL ==="
      exit 1
    fi
  fi

  # Validate command starts with a trusted prefix to prevent arbitrary execution
  # of untrusted third-party scenario content.
  case "$VERIFY_CMD" in
    docker\ *|grep\ *|node\ *|bash\ *|echo\ *|test\ *)
      ;;
    *)
      echo "Error: verification_command has untrusted prefix. Allowed: docker, grep, node, bash, echo, test"
      echo "Command: $VERIFY_CMD"
      exit 1
      ;;
  esac

  echo "Verification command: $VERIFY_CMD"
  echo ""
  echo "Running verification..."

  # Capture output; allow non-zero exit so set -e does not abort before we can
  # check the stdout marker (same logic as agents/verifier/index.js:runVerification).
  # Commands of the form `... && echo PASS || echo FAIL` exit 0 even on failure
  # because `echo FAIL` succeeds — exit code alone is not a reliable signal.
  set +e
  VERIFY_OUTPUT=$(bash -c "$VERIFY_CMD" 2>&1)
  set -e
  echo "$VERIFY_OUTPUT"

  LAST_LINE=$(echo "$VERIFY_OUTPUT" | awk 'NF' | tail -1 | xargs)

  echo ""
  if [ "$LAST_LINE" = "PASS" ]; then
    echo "=== PASS ==="
    exit 0
  else
    echo "=== FAIL ==="
    exit 1
  fi
fi
