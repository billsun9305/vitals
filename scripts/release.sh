#!/usr/bin/env bash
# Cut a release: bump every version the repository carries, move the
# changelog's Unreleased section under the new version, commit and tag.
# The push stays manual.
#
#   scripts/release.sh 0.2.0            # commit "release: 0.2.0" + annotated tag v0.2.0
#   scripts/release.sh --dry-run 0.2.0  # print the diff the edits would make; touch nothing
#
# A version is MAJOR.MINOR.PATCH, optionally with a hyphen suffix
# (0.2.0-beta.1), which .github/workflows/release.yml publishes as a
# pre-release that installed apps never see.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo_url="https://github.com/billsun9305/vitals"

dry_run=0
if [ "${1:-}" = "--dry-run" ]; then
  dry_run=1
  shift
fi
version="${1:?usage: release.sh [--dry-run] <x.y.z>}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "not a version: $version (want MAJOR.MINOR.PATCH, optionally -suffix)" >&2
  exit 1
fi

# --- the three edits, each on a file path so the dry run can point them at copies ---

bump_cargo() { # <Cargo.toml>
  # The first `version = "..."` line is [workspace.package]'s; both crates
  # inherit it with `version.workspace = true`, so it is the only one.
  perl -0pi -e 's/^version = "[^"]*"/version = "'"$version"'"/m' "$1"
}

bump_plist() { # <Info.plist>
  local build
  build="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$1")"
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$1"
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $((build + 1))" "$1"
}

bump_changelog() { # <CHANGELOG.md>
  local today prev new_link
  today="$(date +%Y-%m-%d)"
  grep -q '^## \[Unreleased\]' "$1" || { echo "$1 has no '## [Unreleased]' section" >&2; exit 1; }
  # A fresh, empty Unreleased heading above the section being released.
  perl -0pi -e 's/^## \[Unreleased\]\n/## [Unreleased]\n\n## ['"$version"'] - '"$today"'\n/m' "$1"
  # Link references at the bottom. The first release has nothing to compare
  # against, so it links to its tag; later ones compare with the previous tag.
  prev="$(sed -nE 's|^\[Unreleased\]: .*/compare/v(.*)\.\.\.HEAD$|\1|p' "$1")"
  if [ -n "$prev" ]; then
    new_link="[$version]: $repo_url/compare/v$prev...v$version"
  else
    new_link="[$version]: $repo_url/releases/tag/v$version"
  fi
  perl -0pi -e 's|^\[Unreleased\]: .*$|[Unreleased]: '"$repo_url"'/compare/v'"$version"'...HEAD\n'"$new_link"'|m' "$1"
}

# --- dry run: copies, diffs, exit ---

if [ "$dry_run" = 1 ]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  cp "$root/Cargo.toml" "$tmp/Cargo.toml"
  cp "$root/resources/Info.plist" "$tmp/Info.plist"
  cp "$root/CHANGELOG.md" "$tmp/CHANGELOG.md"
  bump_cargo "$tmp/Cargo.toml"
  bump_plist "$tmp/Info.plist"
  bump_changelog "$tmp/CHANGELOG.md"
  diff -u "$root/Cargo.toml" "$tmp/Cargo.toml" || true
  diff -u "$root/resources/Info.plist" "$tmp/Info.plist" || true
  diff -u "$root/CHANGELOG.md" "$tmp/CHANGELOG.md" || true
  exit 0
fi

# --- the real thing ---

cd "$root"
branch="$(git rev-parse --abbrev-ref HEAD)"
[ "$branch" = main ] || { echo "release from main, not from $branch" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "the working tree is not clean" >&2; exit 1; }
if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
  echo "tag v$version already exists" >&2
  exit 1
fi

bump_cargo Cargo.toml
cargo update -w --offline
bump_plist resources/Info.plist
bump_changelog CHANGELOG.md

git add Cargo.toml Cargo.lock resources/Info.plist CHANGELOG.md
git commit -q -m "release: $version"
git tag -a "v$version" -m "Vitals $version"

echo "released $version locally: commit $(git rev-parse --short HEAD), tag v$version"
echo "to publish:"
echo "  git push --follow-tags"
