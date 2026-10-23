#!/usr/bin/env bash
# Prints the CHANGELOG.md section for a version (used as the GitHub release notes).
#   scripts/release-notes.sh 1.2.0
set -euo pipefail
version="${1:?usage: release-notes.sh <version>}"
changelog="$(dirname "$0")/../CHANGELOG.md"

notes=$(awk -v version="$version" '
  /^## \[/ {
    if (found) exit
    if (index($0, "## [" version "]") == 1) { found = 1; next }
  }
  # Link definitions at the end of the file belong to no section.
  /^\[[^]]+\]: / { if (found) exit }
  found { print }
' "$changelog")

# Trim leading/trailing blank lines.
notes=$(printf '%s\n' "$notes" | sed -e '/./,$!d' | sed -e ':a' -e '/^\n*$/{$d;N;ba' -e '}')
if [ -z "$notes" ]; then
  echo "no CHANGELOG.md section for $version" >&2
  exit 1
fi
printf '%s\n' "$notes"
