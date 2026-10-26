#!/usr/bin/env bash
# Prepares a release: moves the "Unreleased" changelog entries under the new version, commits
# and creates the tag, which is the version of the build. Nothing is pushed.
#   scripts/release.sh 1.2.0          # or 1.3.0-rc.1 for a prerelease
#   git push origin HEAD v1.2.0       # starts .github/workflows/release.yml
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:?usage: scripts/release.sh <version>}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "not a semantic version: $version" >&2
  exit 1
fi
tag="v$version"
if [ -n "$(git status --porcelain)" ]; then
  echo "the working tree has uncommitted changes" >&2
  exit 1
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "tag $tag already exists" >&2
  exit 1
fi
if ! awk '/^## \[Unreleased\]/{f=1;next} /^## \[/{f=0} f && /[^[:space:]]/{n=1} END{exit !n}' CHANGELOG.md; then
  echo "CHANGELOG.md has no entries under ## [Unreleased]" >&2
  exit 1
fi

current="$(scripts/version.sh)"

today=$(date -u +%Y-%m-%d)
perl -0pi -e "s/^## \[Unreleased\]\n/## [Unreleased]\n\n## [$version] - $today\n/m" CHANGELOG.md
# Comparison links at the end of the file.
perl -0pi -e "s{^\[Unreleased\]: (.*)/compare/v[^.\s]+\.[^.\s]+\.\S+?\.\.\.HEAD\n}{[Unreleased]: \$1/compare/$tag...HEAD\n[$version]: \$1/releases/tag/$tag\n}m" CHANGELOG.md

scripts/release-notes.sh "$version" >/dev/null
cargo check --workspace --locked --quiet

git add CHANGELOG.md
git commit -m "Release $tag"
git tag -a "$tag" -m "Scratchpad $version"
echo "Created $tag ($current -> $version). Push with: git push origin HEAD $tag"
