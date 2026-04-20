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

# Update Homebrew formula version + clear sha256 placeholder (filled in by release CI)
FORMULA="Formula/decipher-cli.rb"
if [ -f "$FORMULA" ]; then
  # Update the tarball URL version
  sed -i.bak "s|/refs/tags/v[0-9]*\.[0-9]*\.[0-9]*/|/refs/tags/v${NEW_VERSION}/|g" "$FORMULA"
  # Reset sha256 (release CI will patch it after tarball is published)
  sed -i.bak 's/sha256 "[^"]*"/sha256 ""/' "$FORMULA"
  rm -f "${FORMULA}.bak"
fi

# Commit and tag
git add package.json ${FORMULA:+$FORMULA}
git commit -m "chore: bump version to $NEW_VERSION"
git tag "v$NEW_VERSION"

echo ""
echo "  ✓ Bumped to v$NEW_VERSION"
echo "  ✓ Created commit and tag v$NEW_VERSION"
echo ""
echo "  To publish:"
echo "    git push origin main --tags"
