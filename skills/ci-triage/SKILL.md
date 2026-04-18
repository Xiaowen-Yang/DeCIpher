# CI Triage Domain Knowledge

## Common CI Failure Patterns

### dependency_version_mismatch
- Symptom: `requires-python`, `not in '>=X.Y'`, `Package requires a different Python`
- Look for: version constraint in workflow vs pyproject.toml / setup.py / .tool-versions / .nvmrc
- Fix: align CI version with project minimum version requirement

### ci_config_drift
- Symptom: workflow step fails unexpectedly after a merge, env variable missing
- Look for: recently changed workflow files, new required secrets, removed steps
- Fix: update workflow to match current project state

### cache_or_lockfile_issue
- Symptom: `lockfile is out of date`, `hash mismatch`, package resolution fails
- Look for: uncommitted lockfile changes, CI cache keys not invalidated
- Fix: regenerate lockfile, update cache key, or clear cache

### test_regression
- Symptom: specific test names appear in failure output with FAILED/ERROR markers
- Look for: recently changed code, new dependencies, changed environment
- Fix: determine if test or code is wrong, then fix the appropriate one

## Evidence Collection Priority
1. Exact error message and exit code
2. Which step/job failed
3. Python/Node/runtime version in workflow
4. Relevant config file versions (pyproject.toml, package.json, .tool-versions)
