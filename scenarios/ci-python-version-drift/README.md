# Scenario: ci-python-version-drift

Demonstrates DeCIpher triaging a Python version mismatch between CI workflow and project requirements.

## Problem
The GitHub Actions workflow specifies `python-version: '3.10'`, but `pyproject.toml` requires Python `>=3.11`.

## Expected Classification
`dependency_version_mismatch`

## Expected Fix
Change `python-version: '3.10'` to `python-version: '3.11'` in the workflow file.
