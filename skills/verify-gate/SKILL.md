# Verification Layer Standards

This skill supports DeCIpher's verification layer inside the mission-driven
runtime.

## What Counts as Verification
A step is verified only when:
1. A command was executed
2. The command produced output
3. Exit status or structured PASS/FAIL state was captured
4. The next decision can be justified from that evidence

## Verification Command Types

### build verification
Run the build command that was originally failing.
Expected: exit code 0, no blocking error lines.

### structural verification
When build cannot be run:
- check that the expected file state exists
- use grep or diff to confirm the intended change

### smoke verification
Run the smallest command that proves the service starts.
Example: `docker run --rm <image> echo ok`

### doctor verification
Run `node bin/decipher doctor` to confirm required tools are present.

## Stop Conditions
Return `NEEDS_HUMAN_REVIEW` or an equivalent mission stop when:
- triage confidence is too low
- the same patch is attempted twice
- verification fails with the same blocking evidence repeatedly
- the proposed patch scope is too large for safe autonomous execution
