# Scenario: docker-copy-path-bug

Demonstrates DeCIpher triaging a Docker COPY path error.

## Problem
Dockerfile references `COPY src/ .` but the `src/` directory does not exist in the build context.
The scenario now includes a minimal `package.json` and `index.js` directly in the
build context so that the repaired Dockerfile can complete a real rebuild.

## Expected Classification
`path_or_copy_error`

## Expected Fix
Change `COPY src/ .` to `COPY . .`
