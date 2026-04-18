# Verification Standards Domain Knowledge

## What Counts as Verification
A fix is verified ONLY when:
1. A command was executed (exact command shown)
2. The command produced output (key lines captured)
3. The exit code was captured
4. A PASS or FAIL conclusion is stated

"It should work now" is NOT verification.

## Verification Command Types

### build verification
Run the build command that was originally failing.
Expected: exit code 0, no error lines in output.

### structural verification
When build cannot be run (e.g. CI workflow changes):
- Check that fixed file matches expected state
- Use grep or diff to confirm the change was applied correctly

### smoke verification
Run a minimal command to confirm the service starts.
Example: `docker run --rm <image> echo ok`

### doctor verification
Run `node bin/decipher doctor` to confirm all required tools are present.

## Iteration Stop Conditions
Stop the repair loop early (return NEEDS_HUMAN_REVIEW) if:
- Triage confidence < 0.7
- Same patch attempted twice
- Verification fails with identical error text
- Patch touches more than 2 files
