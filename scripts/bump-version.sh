#!/usr/bin/env bash
# Bump the version in package.json, commit, and tag.
#
# Usage:
#   ./scripts/bump-version.sh patch      # 0.1.0 → 0.1.1
#   ./scripts/bump-version.sh minor      # 0.1.0 → 0.2.0
#   ./scripts/bump-version.sh major      # 0.1.0 → 1.0.0
#   ./scripts/bump-version.sh 1.2.3      # explicit version

set -euo pipefail

LEVEL="${1:-}"

if [ -z "$LEVEL" ]; then
  echo "Usage: $0 <patch|minor|major|x.y.z>"
  exit 1
fi

# Ensure working tree is clean
if [ -n "$(git status --porcelain)" ]; then
  echo "Error: working tree is not clean. Commit or stash changes first."
  exit 1
fi

# Bump version in package.json (no automatic git tag from npm)
npm version "$LEVEL" --no-git-tag-version

# Read the new version
NEW_VERSION=$(node -p "require('./package.json').version")

# Commit and tag
git add package.json
git commit -m "chore: bump version to $NEW_VERSION"
git tag "v$NEW_VERSION"

echo ""
echo "  ✓ Bumped to v$NEW_VERSION"
echo "  ✓ Created commit and tag v$NEW_VERSION"
echo ""
echo "  To publish:"
echo "    git push origin main --tags"
