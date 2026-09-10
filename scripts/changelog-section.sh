#!/usr/bin/env bash
# Print the body of one version's section of CHANGELOG.md, without its
# heading: every line between `## [<version>]` and the next `## [` heading,
# with leading and trailing blank lines removed. Exits 1 when the section
# is missing or empty, so a release with no notes fails before it exists.
#
#   scripts/changelog-section.sh 0.2.0
#   scripts/changelog-section.sh Unreleased
set -euo pipefail

version="${1:?usage: changelog-section.sh <version|Unreleased>}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# `index(...) == 1` rather than a regex, so a version's dots are literal.
body="$(awk -v v="$version" '
  /^## \[/ {
    if (found) exit
    found = (index($0, "## [" v "]") == 1)
    next
  }
  found { lines[n++] = $0 }
  END {
    start = 0; end = n
    while (start < end && lines[start] ~ /^[[:space:]]*$/) start++
    while (end > start && lines[end - 1] ~ /^[[:space:]]*$/) end--
    for (i = start; i < end; i++) print lines[i]
  }
' "$root/CHANGELOG.md")"

if [ -z "$body" ]; then
  echo "CHANGELOG.md has no section for [$version], or it is empty" >&2
  exit 1
fi
printf '%s\n' "$body"
