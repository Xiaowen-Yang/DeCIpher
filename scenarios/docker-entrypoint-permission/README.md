# Scenario: docker-entrypoint-permission

Demonstrates DeCIpher triaging a Docker container runtime permission error.

## Problem
The Dockerfile copies `entrypoint.sh` into the image but does not set execute permission. When the container starts, it fails with `permission denied` because the entrypoint script is not executable (mode 644).

## Expected Classification
`permission_or_executable_error`

## Expected Fix
Add `RUN chmod +x /entrypoint.sh` in the Dockerfile before the ENTRYPOINT instruction.
