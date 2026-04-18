# Scenario: env-missing-node

Demonstrates DeCIpher's doctor/bootstrap capability for missing runtime dependencies.

## Problem
Node.js is not installed on the system, but the project requires Node.js >= 18 (specified in `.nvmrc` and `package.json` engines).

## Expected Classification
`missing_env_or_secret_contract`

## Expected Fix
Install Node.js >= 18 using fnm, nvm, or direct download. This is NOT auto-fixable — DeCIpher provides instructions, not automatic installation.

## Demo Flow
1. `decipher doctor` detects missing Node.js
2. `decipher bootstrap` provides install instructions
3. User installs Node.js
4. `decipher doctor` confirms fix
