# Docker Repair Subsystem Knowledge

This skill supports DeCIpher's mission runtime when the selected subsystem is
repair and the target is Docker-related.

## Common Docker Failure Patterns

### path_or_copy_error
- Symptom: `stat <path>: file does not exist`, `failed to copy files`
- Look for: COPY source paths in Dockerfile vs actual directory structure
- Also check: .dockerignore may be excluding required files
- Fix: correct the COPY source path to match actual build context

### permission_or_executable_error
- Symptom: `permission denied`, `exec user process caused`, `OCI runtime create failed`
- Look for: entrypoint.sh or scripts without execute permission, missing `chmod +x`
- Fix: add `RUN chmod +x /path/to/script` before ENTRYPOINT in Dockerfile

### docker_entrypoint_runtime_error
- Symptom: container exits immediately, `Error response from daemon: container exited with non-zero code`
- Look for: entrypoint script errors, wrong CMD format, missing dependencies at runtime
- Fix: inspect entrypoint script for runtime errors, verify CMD format

### healthcheck_startup_failure
- Symptom: `unhealthy`, `health check failed`, container removed after timeout
- Look for: HEALTHCHECK command, service startup time vs interval, port binding
- Fix: increase healthcheck start period, fix the health endpoint, or correct the port

## Dockerfile Best Practices
- COPY paths are relative to build context (the directory passed to `docker build`)
- ADD and COPY source paths cannot start with `../`
- Files excluded by .dockerignore cannot be COPYed even if they exist
- RUN commands execute in the image, not the host
