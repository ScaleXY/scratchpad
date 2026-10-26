#!/usr/bin/env bash
# Prints the version of a build, from its git tag (same rules as crates/version.rs):
#   SCRATCHPAD_VERSION set     that version (CI; a leading `v` is dropped)
#   HEAD tagged v1.2.0         1.2.0
#   3 commits after v1.2.0     1.2.0+3.gabc1234
#   no git or no tag           0.0.0
set -euo pipefail

if [[ -n "${SCRATCHPAD_VERSION:-}" ]]; then
  echo "${SCRATCHPAD_VERSION#v}"
  exit 0
fi
if describe="$(git -C "$(dirname "$0")" describe --tags --long --match 'v[0-9]*' 2>/dev/null)"; then
  hash="${describe##*-}"
  rest="${describe%-*}"
  count="${rest##*-}"
  version="${rest%-*}"
  version="${version#v}"
  if [[ "$count" == 0 ]]; then
    echo "$version"
  else
    echo "$version+$count.$hash"
  fi
else
  echo "0.0.0"
fi
