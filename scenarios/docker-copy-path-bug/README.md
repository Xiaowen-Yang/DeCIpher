# Scenario: docker-copy-path-bug

Demonstrates DeCIpher triaging a Docker COPY path error.

## Problem
Dockerfile references `COPY src/ .` but the `src/` directory does not exist in the build context.

## Expected Classification
`path_or_copy_error`

## Expected Fix
Change `COPY src/ .` to `COPY . .`
