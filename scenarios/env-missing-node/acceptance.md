# Acceptance Criteria — env-missing-node

## Reproduction
- Log: `scenarios/env-missing-node/logs/bootstrap-failure.log`
- Missing: Node.js runtime

## Classification
- Expected: `missing_env_or_secret_contract`

## Fix
- Not auto-fixable
- DeCIpher should provide install instructions for the user's OS

## Verification Command
```bash
node --version >/dev/null 2>&1 && echo PASS || echo FAIL
```

## Pass Criteria
- Classification label matches `missing_env_or_secret_contract`
- Doctor output shows Node.js as missing
- Bootstrap output provides OS-specific install instructions
