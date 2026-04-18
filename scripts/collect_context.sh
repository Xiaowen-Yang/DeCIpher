#!/usr/bin/env bash
# Collect project context for triage

echo "=== Git Status ==="
git status --short 2>/dev/null || echo "(not a git repo)"

echo ""
echo "=== Recent Commits ==="
git log --oneline -5 2>/dev/null || echo "(no git log)"

echo ""
echo "=== Changed Files ==="
git diff --name-only HEAD 2>/dev/null || echo "(no changes)"

echo ""
echo "=== Dockerfiles ==="
find . -name 'Dockerfile*' -not -path '*/node_modules/*' -not -path '*/.git/*' 2>/dev/null

echo ""
echo "=== CI Workflows ==="
find . -path '*/.github/workflows/*.yml' -not -path '*/node_modules/*' 2>/dev/null
