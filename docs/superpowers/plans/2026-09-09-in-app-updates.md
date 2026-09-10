# In-app updates and distribution — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `vitals` as a notarized disk image on GitHub Releases that a new user installs by dragging, that starts at login on its own, and that tells every installed menu bar copy when a newer version exists and installs it in one click.

**Architecture:** A release script bumps every version the repository carries and tags; a tag-triggered workflow builds, signs, notarizes and publishes the tarball, the disk image and `SHA256SUMS`. In the app, a new `update` module does the pure parsing (`release.rs`), the HTTP (`http.rs`, `NSURLSession`), the signature question (`codesign.rs`), the stage/verify/unpack/swap (`install.rs`) and the relaunch helper (`relaunch.rs`); `checker.rs` holds the state machine. The tray's controller gains an update file (`tray/update_ui.rs`) that owns the timers, the menu rows, the menu bar badge and the alerts, and a login item module (`tray/login_item.rs`) over `SMAppService` replaces the LaunchAgent.

**Tech Stack:** Rust 2021 · `objc2` 0.6.4 + `objc2-foundation`/`objc2-app-kit` 0.3.2 (existing) · `objc2-security` 0.3.2 · `objc2-service-management` 0.3.2 · `objc2-core-foundation` 0.3.2 · `flate2` 1 · `tar` 0.4 · `sha2` 0.10 · `serde_json` (existing) · `tiny_http` 0.12 (existing, test fixture) · `tempfile` 3 (dev) · bash + perl + PlistBuddy (scripts) · GitHub Actions on `macos-15`

**Spec:** `docs/superpowers/specs/2026-09-09-in-app-updates-design.md`. Section numbers below (§1–§6) refer to it.

**Repo:** `billsun9305/vitals` · **Target:** macOS 14+ on `aarch64` only

---

## Global Constraints

Every task's requirements implicitly include this section.

- **Platform and toolchain:** macOS on `aarch64` only. Rust edition 2021, `rust-version = "1.82"` — every new dependency must build on 1.82 (`sha2` is pinned to `0.10` for this reason; `0.11` needs 1.85). objc2 crates are pinned exactly: `objc2-security = "=0.3.2"`, `objc2-service-management = "=0.3.2"`, `objc2-core-foundation = "=0.3.2"`.
- **`make lint` must be clean:** `cargo clippy --workspace --all-targets --examples -- -D warnings`. Deprecated API use is a warning, so the Security framework is called through the method-style names (`SecCode::copy_self`, `SecCode::copy_static_code`, `SecCode::copy_signing_information`, `SecStaticCode::create_with_path`, `SecStaticCode::check_validity`, `SecRequirement::create_with_string`), never the deprecated free functions.
- **`cargo fmt --all --check` must be clean.** Run `cargo fmt --all` before every commit.
- **Every `unsafe` block carries a `// SAFETY:` comment** naming the contract it upholds (CONTRIBUTING rule 6).
- **Nothing runs while nobody is looking** (CONTRIBUTING rule 2): the only idle cost this feature may add is one `NSTimer` a day with `tolerance = 3600` s. Every `NSURLSession` is created per call and `finishTasksAndInvalidate`d after it. No thread or socket exists between checks.
- **No subprocess is spawned except our own binary** (`vitals relaunch`, spawned by the installer). HTTP is `NSURLSession`; unpacking is `flate2` + `tar`; hashing is `sha2`.
- **Main-thread discipline:** every ivar of `Controller` is main-thread-only. A callback that arrives on another thread (a notification posted from a worker thread) does nothing but `performSelectorOnMainThread:` — the exact pattern of `powerChanged:` / `powerChangedOnMain` in `crates/app/src/tray/controller.rs`.
- **Cross-thread values are plain Rust:** a block or closure that runs on a Foundation queue copies what it needs into `Vec<u8>`, `String`, `PathBuf`, `i32` and sends that down an `mpsc` channel or into an `Arc<Mutex<Option<_>>>`. No `Retained<...>` crosses a thread (see `tray/child.rs`'s completion handler for the existing precedent).
- **Exact user-visible strings** (§2–§4, §6): menu bar badge `" ●"` (space, U+25CF) in `NSColor::controlAccentColor()`; top row `● Update to Vitals <x.y.z>…` / `Installing…`; `Check for Updates…` / `Checking…`; `Start at Login` / `Start at Login — approve in System Settings…`; alert titles `Vitals <x.y.z> is available`, `You're up to date`, `Couldn't check for updates`, `Couldn't install Vitals <x.y.z>`; alert buttons in order `Install and Relaunch`, `Later`, `View Release`; notes truncated at 1,500 characters with `…`.
- **Exact schedule** (§2): first check 30 s after the tray starts (one-shot `NSTimer`), then every 24 h (`86400` s) with `tolerance = 3600` s. Timers added to the run loop in `NSRunLoopCommonModes`, like `sync_timer`.
- **Exact request** (§2): `GET <source.api_latest>` with headers `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28`, `User-Agent: vitals/<CARGO_PKG_VERSION>`; `timeoutIntervalForRequest = 10` s. Nothing else is sent.
- **Exact asset contract** (§1): `Vitals-<version>-arm64.tar.gz` (exactly one top-level entry, `Vitals.app/`), `Vitals-<version>.dmg`, `SHA256SUMS` (`<hex>  <filename>`, two spaces); download URLs `https://github.com/billsun9305/vitals/releases/download/v<version>/<name>`.
- **The feature exists only in a bundle:** when `NSBundle::mainBundle().bundleURL()` does not end in `.app`, no timers are created, no update or login items are added to the menu, and `register_once` is not called.
- **`--update-source <url>`** is accepted only for `http://127.0.0.1:<port>/` and `http://localhost:<port>/` (trailing slash required); anything else is an error before AppKit starts.
- **Never in a test:** launching an app, spawning `vitals relaunch` against a real bundle (the helper's own tests run it against paths that cannot be opened), touching `/Applications`, `~/Library/LaunchAgents`, `launchctl`, or the login item registry. Integration tests work on temporary bundles under `tempfile::tempdir()`.
- **Secrets are never handled by tooling:** `scripts/release-secrets.sh` (already committed) is run by the maintainer. No task reads, prints or sets a certificate, password or key.
- **Commit after every task**, `cargo fmt` first, using the message given in that task's final step, with the trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. `git add <paths>` only — never `git add -A`.

---

## Part 0 — Decisions the spec left open, and two corrections

Every AppKit, Foundation, Security and ServiceManagement name in this plan was checked against the crate sources in `~/.cargo/registry` before the plan was written. Where the spec's sketch and the real API differ, the real API wins, as listed here.

| # | Spec said | Verified reality | Consequence |
|---|---|---|---|
| 1 | `sha2` (version unstated) | `sha2 0.11.0` has `rust-version = 1.85`; the workspace floor is `1.82` | `sha2 = "0.10"`. Its `finalize()` returns a `GenericArray<u8, U32>`, which is `Into<[u8; 32]>`. Task 3. |
| 2 | `SecCodeCopySelf`, `SecCodeCopySigningInformation`, `SecStaticCodeCheckValidity`, … | In `objc2-security 0.3.2` those free functions carry `#[deprecated = "renamed to …"]`, which `-D warnings` turns into a build failure | Use `SecCode::copy_self(flags, out)`, `SecCode::copy_static_code(&self, flags, out)`, `SecCode::copy_signing_information(&SecStaticCode, flags, out)`, `SecStaticCode::create_with_path(&CFURL, flags, out)`, `SecRequirement::create_with_string(&CFString, flags, out)`, `SecStaticCode::check_validity(&self, flags, Option<&SecRequirement>)`. All `unsafe`, all return `OSStatus`, all write through a `NonNull<*mut T>` / `NonNull<*const T>` out-pointer. Task 5. |
| 3 | `NSAttributedString` from the title | `NSAttributedString::from_nsstring` exists but is immutable; attributes are added with `NSMutableAttributedString::addAttribute_value_range(&NSAttributedStringKey, &AnyObject, NSRange)` (unsafe) | Build an `NSMutableAttributedString`, add the font and `NSColor::labelColor()` over the whole range and `controlAccentColor()` over the badge range, then pass it (it derefs to `NSAttributedString`). Ranges are UTF-16 code units, so `status_item` computes them with `encode_utf16().count()`. Task 9. |
| 4 | "`performSelectorOnMainThread:` from the worker" | `Retained<Controller>` is `MainThreadOnly` and cannot be moved to the worker thread | The worker leaves its result in an `Arc<Mutex<Option<_>>>` and posts an `NSNotification` (`NSNotificationCenter::defaultCenter().postNotificationName_object`, unsafe). The controller observes it; the observer method runs on the worker's thread and only hops, exactly like `powerChanged:`. Tasks 8, 10. |
| 5 | `Staged { dir, app }` | Test case 1 wants to know the signature check was skipped, and cases 2–5 want to assert the staging directory is gone after a failure that never returned a `Staged` | `Staged` also carries `signature_checked: bool` (accessor), and `prepare` is split into `prepare(source, release, app_url)` (creates the `NSItemReplacementDirectory`) over `prepare_in(dir, source, release)` (caller-chosen directory, used by tests 2–5). Task 6. |
| 6 | The helper "waits … then launches through `NSWorkspace`" | `openApplicationAtURL_configuration_completionHandler` answers through a completion block on a LaunchServices queue; a process with no run loop may never see it | The helper pumps `NSRunLoop::mainRunLoop().runUntilDate(...)` in 100 ms slices until the block has sent its result down a channel, or 30 s pass. Task 7. |
| 7 | `PREFIX` unstated; README says `make install-app PREFIX=$HOME/.local` | `Makefile`'s `PREFIX ?= /usr/local` makes the documented no-`sudo` path opt-in and the failing path the default | `PREFIX ?= $(HOME)/.local`. `make install-app` with no arguments needs no `sudo`; `PREFIX=/usr/local` is still accepted. Task 11. |
| 8 | Update rows "inserted or removed in `menuWillOpen:`" | `NSMenuItem`s must exist before `init` to live in ivars, but their target (`self`) exists only after | The items are created in `new` before `init` and stored in the ivars; `setTarget`/`setAction` run after `init`, the way the existing *Open Dashboard* item's do. `menuWillOpen:` inserts, removes and retitles from the current state. Task 8. |
| 9 | Controller "on the controller" | `controller.rs` is 525 lines; the update and login logic is another ~300 | Selectors stay in `define_class!` in `controller.rs` (they must), each one line; the bodies live in `tray/update_ui.rs` as a second `impl Controller` block. The ivars that file needs are `pub(super)`. Tasks 8–11. |
| 10 | `Command::LoginItem { action: On \| Off \| Status }` | clap derives a nested subcommand for this | `vitals login-item on` / `off` / `status` via `#[command(subcommand)] action: LoginItemAction`. Task 11. |

Two further choices, made on judgement:

- **The tarball is built with `COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata`.** macOS `bsdtar` otherwise emits `._Vitals.app` AppleDouble entries and extended-attribute headers; the first would break the "exactly one top-level entry" rule and the second is exactly the quarantine flag the design promises never arrives. `tar::Archive` ignores xattr headers by default anyway; the flags make the archive honest on both ends.
- **`resources/Info.plist` is normalised once with `plutil -convert xml1`** (Task 1) so that `PlistBuddy`, which rewrites the whole file on every `Set`, produces two-line diffs on release day instead of reformatting the file.

---

## Part A — File structure

```
scripts/
├── bundle.sh                     # MODIFY: SIGN_IDENTITY, hardened runtime, comment rewritten (T2)
├── changelog-section.sh          # NEW: prints one CHANGELOG section's body (T1)
├── release.sh                    # NEW: bump versions, changelog, commit, tag; --dry-run (T1)
└── release-secrets.sh            # exists; maintainer-run; untouched
.github/workflows/
├── ci.yml                        # MODIFY: release.sh dry-run step (T1)
└── release.yml                   # NEW: tag -> build, sign, notarize, DMG, sums, publish (T2)
resources/
├── Info.plist                    # MODIFY: normalised formatting (T1)
└── com.billsun.vitals.plist      # DELETE (T11)
Makefile                          # MODIFY: PREFIX default, install-app, uninstall-app (T11)
Cargo.toml                        # MODIFY: workspace deps (T3)
crates/app/Cargo.toml             # MODIFY: deps + features (T3)
crates/app/src/
├── lib.rs                        # MODIFY: pub mod update (T3)
├── main.rs                       # MODIFY: relaunch, login-item, --update-source (T7, T8, T11)
├── cli.rs                        # MODIFY: Relaunch, LoginItem, update_source (T7, T8, T11)
├── update/
│   ├── mod.rs                    # NEW (T3, grows per task)
│   ├── release.rs                # NEW: Version, Source, Release, parse_latest, parse_sums, expected_sum, UpdateError (T3)
│   ├── http.rs                   # NEW: fetch, download over NSURLSession (T4)
│   ├── codesign.rs               # NEW: team_id_of_self, verify_team, requirement_for (T5)
│   ├── install.rs                # NEW: prepare, prepare_in, Staged, Helper, run (T6)
│   ├── relaunch.rs               # NEW: the `vitals relaunch` body (T7)
│   └── checker.rs                # NEW: CheckOutcome, UpdateState, check_now, spawn_check (T8)
├── window/mod.rs                 # MODIFY: pub(crate) mod parent (T7)
├── window/parent.rs              # MODIFY: pub(crate) fn wait_for_exit (T7)
└── tray/
    ├── mod.rs                    # MODIFY: run(source), pub mod login_item, pub(super) mod update_ui (T8, T11)
    ├── controller.rs             # MODIFY: ivars, selectors, menu, set_title (T8, T9, T10, T11)
    ├── update_ui.rs              # NEW: the controller's update + login behaviour (T8–T11)
    ├── status_item.rs            # MODIFY: UPDATE_BADGE, TitleState (T9)
    └── login_item.rs             # NEW: SMAppService wrapper, menu_state, register_once (T11)
crates/app/tests/
├── update.rs                     # NEW: installer end-to-end against a tiny_http fixture (T6)
└── relaunch.rs                   # NEW: the helper as a subprocess (T7)
README.md, CONTRIBUTING.md, SECURITY.md, CHANGELOG.md   # MODIFY (T2, T12)
docs/budget.md                    # MODIFY: measurements (T13, coordinator)
```

Module responsibilities, one line each:

- `update::release` — pure. Knows what a version, a source and a release are; parses GitHub's JSON and `SHA256SUMS`; owns `UpdateError`. Depends on `serde_json` only.
- `update::http` — two blocking functions over `NSURLSession`. Depends on Foundation.
- `update::codesign` — two questions for the Security framework. Depends on `objc2-security`, `objc2-core-foundation`.
- `update::install` — stage, download, verify, unpack, check, spawn, swap. Depends on `http`, `codesign`, `release`, Foundation, `flate2`, `tar`, `sha2`.
- `update::relaunch` — wait for a pid, open a bundle. Depends on `window::parent`, AppKit's `NSWorkspace`.
- `update::checker` — one check and the state it produces; the worker thread. Depends on `http`, `release`.
- `tray::update_ui` — everything the controller does about updates and the login item: timers, rows, badge, alerts, install sequencing. Depends on all of the above plus AppKit.
- `tray::login_item` — `SMAppService` wrapper plus the pure `menu_state`.

---

## Part B — Tasks

### Task 1: Release scripts and the CI dry run

Section §1 (`scripts/release.sh`, `scripts/changelog-section.sh`, "CI runs the dry run on every push") and §5 "Scripts".

**Files:**
- Create: `scripts/changelog-section.sh`
- Create: `scripts/release.sh`
- Modify: `resources/Info.plist` (normalise only; no value changes)
- Modify: `.github/workflows/ci.yml` (one step, between `Test` and `Bundle`)

**Interfaces:**
- Produces: `scripts/changelog-section.sh <version|Unreleased>` prints the section body to stdout, exit 1 when missing or empty. Used by `release.yml` (Task 2).
- Produces: `scripts/release.sh [--dry-run] <x.y.z>`.

- [ ] **Step 1: Normalise `resources/Info.plist`**

`PlistBuddy` rewrites the whole file on every `Set`. Do the rewrite once now so the release-day diff is two lines:

```bash
plutil -convert xml1 resources/Info.plist
git diff --stat resources/Info.plist
```

Expected: the file is re-indented with tabs; `plutil -lint resources/Info.plist` says `OK`; `/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' resources/Info.plist` prints `0.1.0` and `Print :CFBundleVersion` prints `1`.

- [ ] **Step 2: Write `scripts/changelog-section.sh`**

```bash
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
```

- [ ] **Step 3: Check it by hand**

```bash
chmod +x scripts/changelog-section.sh
scripts/changelog-section.sh Unreleased | head -3
scripts/changelog-section.sh 9.9.9; echo "exit $?"
```

Expected: the first prints `The first release. Everything below is new.` as its first line (no blank line above it, no `## [Unreleased]` heading). The second prints the error to stderr and `exit 1`.

- [ ] **Step 4: Write `scripts/release.sh`**

```bash
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
```

- [ ] **Step 5: Check the dry run**

```bash
chmod +x scripts/release.sh
scripts/release.sh --dry-run 9.9.9
```

Expected, in one output: `-version = "0.1.0"` / `+version = "9.9.9"`; in the plist diff `-<string>0.1.0</string>` / `+<string>9.9.9</string>` and `-<string>1</string>` / `+<string>2</string>`; in the changelog diff `+## [9.9.9] - 2026-…` directly under `## [Unreleased]`, and at the bottom `-[Unreleased]: https://github.com/billsun9305/vitals/commits/main` replaced by `+[Unreleased]: https://github.com/billsun9305/vitals/compare/v9.9.9...HEAD` and `+[9.9.9]: https://github.com/billsun9305/vitals/releases/tag/v9.9.9`. `git status --short` afterwards shows only the files this task created or normalised — the dry run touched nothing.

- [ ] **Step 6: Check the refusals**

```bash
scripts/release.sh 9.9; echo "exit $?"
scripts/release.sh 9.9.9; echo "exit $?"
```

Expected: the first says `not a version: 9.9`, exit 1. The second refuses with either `release from main, not from <branch>` (you are on a task branch) or `the working tree is not clean` — exit 1 either way, and `git log -1` is unchanged.

- [ ] **Step 7: Add the CI step**

In `.github/workflows/ci.yml`, after the `Test` step and before `Bundle`:

```yaml
      # The release script is never run by CI, but a broken one is found
      # here rather than on tag day: the dry run applies its edits to copies
      # and prints the diff, and the notes extractor must find the
      # Unreleased section. Both must print something.
      - name: Release script dry run
        run: |
          scripts/release.sh --dry-run 9.9.9 | tee /dev/stderr | grep -q '9\.9\.9'
          scripts/changelog-section.sh Unreleased | grep -q .
```

- [ ] **Step 8: Commit**

```bash
git add scripts/changelog-section.sh scripts/release.sh resources/Info.plist .github/workflows/ci.yml
git commit -m "build: release script, changelog extractor, CI dry run

scripts/release.sh is the one way a version is cut: it bumps the workspace
version, CFBundleShortVersionString and CFBundleVersion, moves the
changelog's Unreleased section under the new version, commits and tags.
--dry-run applies the edits to copies and prints the diff, which CI now
runs on every push. resources/Info.plist is normalised once so PlistBuddy's
rewrite on release day is a two-line diff.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Signing in `bundle.sh` and the release workflow

Section §1 (`.github/workflows/release.yml`, "Secrets", "The contract the app relies on") and the README "Releasing a new version" rewrite.

**Files:**
- Modify: `scripts/bundle.sh`
- Create: `.github/workflows/release.yml`
- Modify: `README.md` — replace the whole `### Releasing a new version` section (currently lines 409–421)

**Interfaces:**
- Consumes: `scripts/changelog-section.sh` (Task 1).
- Produces: `SIGN_IDENTITY` environment variable honoured by `make bundle`; the three release assets named per the Global Constraints.

- [ ] **Step 1: Rewrite `scripts/bundle.sh`**

```bash
#!/usr/bin/env bash
# Pack target/release/vitals into dist/Vitals.app and sign it.
#
# SIGN_IDENTITY selects the signature. Unset or "-" is ad-hoc: what a local
# build and a fork without secrets get, and what `make install-app`
# installs. Any other value is a codesign identity such as
# "Developer ID Application: Name (TEAMID)"; the bundle is then signed
# with the hardened runtime and a trusted timestamp so that
# .github/workflows/release.yml can notarize it. There is no entitlements
# file, because the app needs no entitlement.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app="$root/dist/Vitals.app"
identity="${SIGN_IDENTITY:--}"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/resources/Info.plist" "$app/Contents/Info.plist"
cp "$root/target/release/vitals" "$app/Contents/MacOS/vitals"
if [ -f "$root/resources/AppIcon.icns" ]; then
  cp "$root/resources/AppIcon.icns" "$app/Contents/Resources/"
fi

if [ "$identity" = "-" ]; then
  codesign --force --deep --sign - "$app"
else
  codesign --force --deep --options runtime --timestamp --sign "$identity" "$app"
fi
codesign --verify --verbose "$app"
echo "built $app (signed as: $identity)"
```

- [ ] **Step 2: Check the ad-hoc path still works**

```bash
cargo build --release && ./scripts/bundle.sh && codesign -dv dist/Vitals.app 2>&1 | grep -E "^Signature|^Identifier"
```

Expected: `Signature=adhoc`, `Identifier=com.billsun.vitals`, and the last line `built …/dist/Vitals.app (signed as: -)`.

- [ ] **Step 3: Write `.github/workflows/release.yml`**

```yaml
name: Release

# A `v*` tag builds, signs, notarizes and publishes a GitHub Release whose
# assets are what the app's updater and a new user both download. Run by
# hand (workflow_dispatch) it does everything except publish and uploads
# the files as a workflow artifact instead, so the signing path can be
# exercised before a tag exists.
#
# Signing and notarization happen only when the repository secret
# MACOS_CERT_P12 is set (see scripts/release-secrets.sh). Without it the
# app is ad-hoc signed and the release notes say so.
on:
  push:
    tags: ["v*"]
  workflow_dispatch:

permissions:
  contents: write

jobs:
  release:
    # Apple Silicon only, the same runner as ci.yml.
    runs-on: macos-15
    steps:
      - uses: actions/checkout@v4

      # The tag, Cargo.toml and Info.plist must agree before anything is
      # built. A manual run takes Cargo.toml's version as the tag's.
      - name: Version
        id: version
        run: |
          cargo_version="$(sed -nE 's/^version = "([^"]+)"/\1/p' Cargo.toml | head -1)"
          plist_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' resources/Info.plist)"
          if [ "$GITHUB_REF_TYPE" = tag ]; then
            version="${GITHUB_REF_NAME#v}"
          else
            version="$cargo_version"
          fi
          if [ "$version" != "$cargo_version" ] || [ "$version" != "$plist_version" ]; then
            echo "version mismatch: tag $version, Cargo.toml $cargo_version, Info.plist $plist_version" >&2
            exit 1
          fi
          echo "version=$version" >> "$GITHUB_OUTPUT"
          case "$version" in
            *-*) echo "prerelease=true" >> "$GITHUB_OUTPUT" ;;
            *)   echo "prerelease=false" >> "$GITHUB_OUTPUT" ;;
          esac
          echo "signed=${{ secrets.MACOS_CERT_P12 != '' }}" >> "$GITHUB_OUTPUT"

      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: npm
          cache-dependency-path: dashboard/package-lock.json

      - name: Dashboard
        working-directory: dashboard
        run: |
          npm ci
          npm run build

      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2

      # The certificate goes into a throwaway keychain that only this job
      # can see; the identity's name is handed to bundle.sh through
      # SIGN_IDENTITY. The keychain password is random and never printed.
      - name: Import signing certificate
        if: steps.version.outputs.signed == 'true'
        env:
          MACOS_CERT_P12: ${{ secrets.MACOS_CERT_P12 }}
          MACOS_CERT_PASSWORD: ${{ secrets.MACOS_CERT_PASSWORD }}
        run: |
          keychain="$RUNNER_TEMP/release.keychain-db"
          keychain_password="$(uuidgen)"
          security create-keychain -p "$keychain_password" "$keychain"
          security set-keychain-settings -lut 21600 "$keychain"
          security unlock-keychain -p "$keychain_password" "$keychain"
          printf '%s' "$MACOS_CERT_P12" | base64 --decode > "$RUNNER_TEMP/cert.p12"
          security import "$RUNNER_TEMP/cert.p12" -k "$keychain" -P "$MACOS_CERT_PASSWORD" \
            -T /usr/bin/codesign -T /usr/bin/security
          rm -f "$RUNNER_TEMP/cert.p12"
          security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain"
          security list-keychains -d user -s "$keychain" login.keychain-db
          identity="$(security find-identity -v -p codesigning "$keychain" \
            | sed -nE 's/.*"(Developer ID Application: [^"]+)".*/\1/p' | head -1)"
          if [ -z "$identity" ]; then
            echo "the imported certificate holds no 'Developer ID Application' identity" >&2
            exit 1
          fi
          echo "SIGN_IDENTITY=$identity" >> "$GITHUB_ENV"

      - name: Bundle
        run: make bundle

      # Notarization needs a zip of the bundle; the ticket is then stapled
      # into the bundle itself, so every artefact made from it carries it.
      - name: Notarize the app
        if: steps.version.outputs.signed == 'true'
        env:
          NOTARY_KEY_P8: ${{ secrets.NOTARY_KEY_P8 }}
          NOTARY_KEY_ID: ${{ secrets.NOTARY_KEY_ID }}
          NOTARY_ISSUER_ID: ${{ secrets.NOTARY_ISSUER_ID }}
        run: |
          printf '%s' "$NOTARY_KEY_P8" > "$RUNNER_TEMP/AuthKey.p8"
          ditto -c -k --keepParent dist/Vitals.app "$RUNNER_TEMP/Vitals.zip"
          xcrun notarytool submit "$RUNNER_TEMP/Vitals.zip" \
            --key "$RUNNER_TEMP/AuthKey.p8" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID" --wait
          xcrun stapler staple dist/Vitals.app

      # The tarball is the updater's asset: exactly one top-level entry,
      # Vitals.app. COPYFILE_DISABLE and --no-xattrs keep bsdtar from adding
      # ._ AppleDouble entries and extended-attribute headers, which would
      # break that rule and carry a quarantine flag respectively.
      # The disk image is the human's download.
      - name: Pack
        env:
          VERSION: ${{ steps.version.outputs.version }}
        run: |
          mkdir -p out
          COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -C dist -czf "out/Vitals-$VERSION-arm64.tar.gz" Vitals.app
          tar -tzf "out/Vitals-$VERSION-arm64.tar.gz" | cut -d/ -f1 | sort -u
          staging="$RUNNER_TEMP/dmg"
          mkdir -p "$staging"
          cp -R dist/Vitals.app "$staging/"
          ln -s /Applications "$staging/Applications"
          hdiutil create -volname "Vitals $VERSION" -srcfolder "$staging" -format UDZO -ov "out/Vitals-$VERSION.dmg"

      - name: Sign and notarize the disk image
        if: steps.version.outputs.signed == 'true'
        env:
          VERSION: ${{ steps.version.outputs.version }}
          NOTARY_KEY_ID: ${{ secrets.NOTARY_KEY_ID }}
          NOTARY_ISSUER_ID: ${{ secrets.NOTARY_ISSUER_ID }}
        run: |
          codesign --sign "$SIGN_IDENTITY" --timestamp "out/Vitals-$VERSION.dmg"
          xcrun notarytool submit "out/Vitals-$VERSION.dmg" \
            --key "$RUNNER_TEMP/AuthKey.p8" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID" --wait
          xcrun stapler staple "out/Vitals-$VERSION.dmg"

      - name: Sums and notes
        env:
          VERSION: ${{ steps.version.outputs.version }}
          SIGNED: ${{ steps.version.outputs.signed }}
        run: |
          (cd out && shasum -a 256 "Vitals-$VERSION-arm64.tar.gz" "Vitals-$VERSION.dmg" > SHA256SUMS)
          cat out/SHA256SUMS
          scripts/changelog-section.sh "$VERSION" > out/notes.md \
            || scripts/changelog-section.sh Unreleased > out/notes.md
          if [ "$SIGNED" != true ]; then
            cat >> out/notes.md <<'NOTE'

---

**This build is not notarized.** It is signed ad hoc, so macOS refuses to open it the first time. To open it: try once, then go to System Settings → Privacy & Security, scroll to the message about Vitals, and click *Open Anyway*.
NOTE
          fi
          cat out/notes.md

      - name: Publish
        if: github.ref_type == 'tag'
        env:
          GH_TOKEN: ${{ github.token }}
          VERSION: ${{ steps.version.outputs.version }}
          PRERELEASE: ${{ steps.version.outputs.prerelease }}
        run: |
          prerelease=""
          if [ "$PRERELEASE" = true ]; then prerelease="--prerelease"; fi
          gh release create "v$VERSION" --title "Vitals $VERSION" --notes-file out/notes.md $prerelease \
            "out/Vitals-$VERSION.dmg" "out/Vitals-$VERSION-arm64.tar.gz" out/SHA256SUMS

      - uses: actions/upload-artifact@v4
        if: github.ref_type != 'tag'
        with:
          name: release-${{ steps.version.outputs.version }}
          path: out/
          if-no-files-found: error
```

- [ ] **Step 4: Rehearse the `Pack` and `Sums and notes` steps locally**

The workflow itself cannot run here; its two shell-only steps can, against the bundle from Step 2:

```bash
rm -rf /tmp/vitals-pack && mkdir -p /tmp/vitals-pack/out && cd /tmp/vitals-pack
V=0.1.0; ROOT=$OLDPWD
COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -C "$ROOT/dist" -czf "out/Vitals-$V-arm64.tar.gz" Vitals.app
tar -tzf "out/Vitals-$V-arm64.tar.gz" | cut -d/ -f1 | sort -u
mkdir -p dmg && cp -R "$ROOT/dist/Vitals.app" dmg/ && ln -s /Applications dmg/Applications
hdiutil create -volname "Vitals $V" -srcfolder dmg -format UDZO -ov "out/Vitals-$V.dmg" | tail -1
(cd out && shasum -a 256 "Vitals-$V-arm64.tar.gz" "Vitals-$V.dmg" > SHA256SUMS && cat SHA256SUMS)
"$ROOT/scripts/changelog-section.sh" "$V" > out/notes.md || "$ROOT/scripts/changelog-section.sh" Unreleased > out/notes.md
head -2 out/notes.md; cd "$ROOT"; rm -rf /tmp/vitals-pack
```

Expected: the `tar -tzf … | sort -u` line prints exactly `Vitals.app` and nothing else; `hdiutil` ends with `created: …/out/Vitals-0.1.0.dmg`; `SHA256SUMS` has two lines of the form `<64 hex>  Vitals-0.1.0-arm64.tar.gz` / `<64 hex>  Vitals-0.1.0.dmg`; `notes.md` starts with `The first release. Everything below is new.`.

- [ ] **Step 5: Validate the workflow file**

```bash
ruby -ryaml -e 'YAML.load_file(".github/workflows/release.yml"); puts "yaml ok"'
grep -c "steps.version.outputs.signed == 'true'" .github/workflows/release.yml
```

Expected: `yaml ok`, and `3` (import, notarize app, notarize DMG are the only steps gated on signing).

- [ ] **Step 6: Rewrite README's "Releasing a new version"**

Replace the section from `### Releasing a new version` up to (not including) `## Development` with:

````markdown
### Releasing a new version

One script cuts a release; one workflow publishes it.

```bash
scripts/release.sh 0.2.0     # bumps Cargo.toml, Info.plist and CHANGELOG.md, commits, tags v0.2.0
git push --follow-tags       # the push is deliberately manual
```

`scripts/release.sh --dry-run 0.2.0` prints the diff without touching
anything; CI runs that dry run on every push. A version with a hyphen
suffix (`0.2.0-beta.1`) becomes a GitHub pre-release, which installed apps
never see.

The tag triggers `.github/workflows/release.yml` on an Apple Silicon
runner. It checks that the tag, `Cargo.toml` and `Info.plist` agree,
builds the dashboard and the bundle, signs and notarizes when the
secrets exist, and publishes a GitHub Release with three assets:

| Asset | Who uses it |
|---|---|
| `Vitals-<version>.dmg` | You. A notarized disk image: open, drag to Applications. |
| `Vitals-<version>-arm64.tar.gz` | The app's updater. Exactly one top-level entry, `Vitals.app/`. |
| `SHA256SUMS` | The updater, to verify the tarball before unpacking it. |

The updater also relies on the bundle's `CFBundleShortVersionString`
equalling the release version, and on a signed release carrying a
Developer ID signature from the same Team ID as the running copy.

**Signing.** Five repository secrets make a release signed and notarized:
`MACOS_CERT_P12` (a Developer ID Application certificate, base64),
`MACOS_CERT_PASSWORD`, `NOTARY_KEY_ID`, `NOTARY_ISSUER_ID` and
`NOTARY_KEY_P8` (an App Store Connect API key). `scripts/release-secrets.sh`
sets all five from your own machine and explains, in its header, how to
create the two things Apple has to issue. Without the secrets the workflow
still publishes a working release, ad-hoc signed, and the release notes
say how to open it. Running the workflow by hand (*Actions → Release →
Run workflow*) does everything except publish, and uploads the three
files as an artifact instead — the way to try the signing path before a
tag exists.

````

- [ ] **Step 7: Commit**

```bash
git add scripts/bundle.sh .github/workflows/release.yml README.md
git commit -m "build: release workflow with Developer ID signing and notarization

A v* tag builds the dashboard and the bundle, imports the Developer ID
certificate into a throwaway keychain when the secrets exist, signs with
the hardened runtime, notarizes and staples the app and the disk image,
and publishes the DMG, the updater's tarball and SHA256SUMS as a GitHub
Release. Without the secrets the same workflow publishes an ad-hoc build
and says so in the notes. bundle.sh takes SIGN_IDENTITY, default ad-hoc.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: `update::release` — versions, sources, releases, sums (pure)

Section §2 "Types (in `release.rs`)" and §5 "Unit (pure)". Also adds every new dependency the later tasks need, so the manifest changes once.

**Files:**
- Modify: `Cargo.toml` (`[workspace.dependencies]`)
- Modify: `crates/app/Cargo.toml` (`[dependencies]`, `[dev-dependencies]`)
- Modify: `crates/app/src/lib.rs`
- Create: `crates/app/src/update/mod.rs`
- Create: `crates/app/src/update/release.rs`

**Interfaces:**
- Produces (used by Tasks 4–10):
  - `pub struct Version { pub major: u32, pub minor: u32, pub patch: u32 }` — `Copy`, `Ord`, `Display` (`0.2.0`); `Version::parse(&str) -> Result<Version, UpdateError>`; `Version::current() -> Version`.
  - `pub struct Source { pub api_latest: String, pub download_base: String }` — `Clone`; `Source::github()`; `Source::loopback(&str) -> Result<Source, UpdateError>`.
  - `pub struct Release { pub version: Version, pub notes: String, pub page_url: String, pub archive_url: String, pub sums_url: String, pub archive_name: String }` — `Clone`, `PartialEq`.
  - `pub fn archive_name(Version) -> String`; `pub const SUMS_NAME: &str = "SHA256SUMS"`; `pub const REPO: &str = "billsun9305/vitals"`.
  - `pub fn parse_latest(&[u8], &Source) -> Result<Release, UpdateError>`; `pub fn parse_sums(&str) -> Vec<(String, String)>`; `pub fn expected_sum(&str, &str) -> Option<[u8; 32]>`.
  - `pub enum UpdateError { BadVersion(String), BadSource(String), BadRelease(String), Http(String), BadChecksum, BadSignature(String), Io(String) }` — `Clone`, `PartialEq`, `Display`, `Error`, `From<std::io::Error>`.

- [ ] **Step 1: Add the dependencies**

In `Cargo.toml`, append to `[workspace.dependencies]`:

```toml
flate2 = "1"
tar = "0.4"
# 0.11 needs Rust 1.85; the workspace floor is 1.82.
sha2 = "0.10"
tempfile = "3"
objc2-security = "=0.3.2"
objc2-service-management = "=0.3.2"
objc2-core-foundation = "=0.3.2"
```

In `crates/app/Cargo.toml`, append to `[dependencies]` (after `block2.workspace = true`):

```toml
# The in-app updater (src/update): gzip + tar to unpack a release, SHA-256
# to verify it before unpacking.
flate2.workspace = true
tar.workspace = true
sha2.workspace = true
# The Security framework, for two questions in update/codesign.rs: which
# Team ID signed the running copy, and does a staged bundle satisfy that
# team's designated requirement. Only the modules those two need.
objc2-security = { workspace = true, features = [
  "CSCommon", "SecCode", "SecStaticCode", "SecRequirement",
] }
# The CF types those Security calls take and return.
objc2-core-foundation = { workspace = true, features = [
  "CFBase", "CFString", "CFURL", "CFDictionary",
] }
# SMAppService, for Start at Login (tray/login_item.rs).
objc2-service-management = { workspace = true, features = ["SMAppService"] }
```

and replace `[dev-dependencies]` with:

```toml
[dev-dependencies]
serde_json.workspace = true
tempfile.workspace = true
```

- [ ] **Step 2: Fetch and build**

```bash
cargo build -p vitals 2>&1 | tail -3
```

Expected: `Finished`. (`Cargo.lock` gains the new crates; it is committed with this task.)

- [ ] **Step 3: Register the module**

`crates/app/src/lib.rs`:

```rust
pub mod cli;
pub mod serve;
pub mod tray;
pub mod update;
pub mod window;
```

`crates/app/src/update/mod.rs`:

```rust
//! In-app updates: finding the latest GitHub Release, downloading and
//! verifying it, swapping it into place and relaunching.
//!
//! The split follows what each file may touch. `release` is pure and owns
//! every decision that can be unit-tested; `http` talks to Foundation's
//! `NSURLSession`; `codesign` to the Security framework; `install` to the
//! file system; `relaunch` to LaunchServices; `checker` runs a check on a
//! worker thread and keeps the state the tray shows. Nothing here touches
//! AppKit — the tray's own `update_ui` does that.

pub mod release;
```

- [ ] **Step 4: Write the failing tests**

Create `crates/app/src/update/release.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub's `releases/latest` shape, reduced to the fields the parser
    /// reads plus a decoy asset (the DMG) it must ignore.
    const LATEST: &str = r#"{
      "tag_name": "v0.2.0",
      "name": "Vitals 0.2.0",
      "draft": false,
      "prerelease": false,
      "html_url": "https://github.com/billsun9305/vitals/releases/tag/v0.2.0",
      "body": "### Added\n- Things.\n",
      "assets": [
        {"name": "Vitals-0.2.0.dmg",
         "browser_download_url": "https://github.com/billsun9305/vitals/releases/download/v0.2.0/Vitals-0.2.0.dmg"},
        {"name": "Vitals-0.2.0-arm64.tar.gz",
         "browser_download_url": "https://github.com/billsun9305/vitals/releases/download/v0.2.0/Vitals-0.2.0-arm64.tar.gz"},
        {"name": "SHA256SUMS",
         "browser_download_url": "https://github.com/billsun9305/vitals/releases/download/v0.2.0/SHA256SUMS"}
      ]
    }"#;

    fn latest_with(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let mut v: serde_json::Value = serde_json::from_str(LATEST).unwrap();
        edit(&mut v);
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn version_parses_with_and_without_the_v() {
        let v = Version { major: 0, minor: 2, patch: 0 };
        assert_eq!(Version::parse("0.2.0"), Ok(v));
        assert_eq!(Version::parse("v0.2.0"), Ok(v));
        assert_eq!(Version::parse(" v0.2.0\n"), Ok(v));
        assert_eq!(v.to_string(), "0.2.0");
    }

    #[test]
    fn version_rejects_anything_but_three_numbers() {
        for bad in ["0.2.0-beta.1", "0.2", "0.2.0.1", "a.b.c", "", "v"] {
            assert!(
                matches!(Version::parse(bad), Err(UpdateError::BadVersion(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn versions_compare_numerically_not_lexically() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.1.1") > v("0.1.0"));
        assert_eq!(v("0.1.0"), v("v0.1.0"));
        assert_eq!(v("0.10.0").to_string(), "0.10.0");
    }

    #[test]
    fn current_version_is_the_crate_version() {
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn latest_json_parses_to_a_release() {
        let release = parse_latest(LATEST.as_bytes(), &Source::github()).unwrap();
        assert_eq!(release.version, Version::parse("0.2.0").unwrap());
        assert_eq!(release.notes, "### Added\n- Things.\n");
        assert_eq!(release.page_url, "https://github.com/billsun9305/vitals/releases/tag/v0.2.0");
        assert_eq!(release.archive_name, "Vitals-0.2.0-arm64.tar.gz");
        assert_eq!(
            release.archive_url,
            "https://github.com/billsun9305/vitals/releases/download/v0.2.0/Vitals-0.2.0-arm64.tar.gz"
        );
        assert_eq!(
            release.sums_url,
            "https://github.com/billsun9305/vitals/releases/download/v0.2.0/SHA256SUMS"
        );
    }

    #[test]
    fn drafts_and_pre_releases_are_rejected() {
        let draft = latest_with(|v| v["draft"] = serde_json::Value::Bool(true));
        let pre = latest_with(|v| v["prerelease"] = serde_json::Value::Bool(true));
        for json in [draft, pre] {
            assert!(matches!(
                parse_latest(&json, &Source::github()),
                Err(UpdateError::BadRelease(_))
            ));
        }
    }

    #[test]
    fn a_suffixed_tag_is_a_bad_release_not_a_bad_version() {
        let json = latest_with(|v| v["tag_name"] = "v0.2.0-rc.1".into());
        assert!(matches!(
            parse_latest(&json, &Source::github()),
            Err(UpdateError::BadRelease(_))
        ));
    }

    #[test]
    fn a_release_missing_an_asset_is_rejected() {
        let no_tarball = latest_with(|v| {
            v["assets"].as_array_mut().unwrap().retain(|a| a["name"] != "Vitals-0.2.0-arm64.tar.gz")
        });
        let no_sums = latest_with(|v| {
            v["assets"].as_array_mut().unwrap().retain(|a| a["name"] != "SHA256SUMS")
        });
        let no_assets = latest_with(|v| v["assets"] = serde_json::Value::Null);
        for json in [no_tarball, no_sums, no_assets] {
            assert!(matches!(
                parse_latest(&json, &Source::github()),
                Err(UpdateError::BadRelease(_))
            ));
        }
    }

    #[test]
    fn an_asset_served_from_elsewhere_is_rejected() {
        // Another host with the right file name.
        let other_host = latest_with(|v| {
            v["assets"][1]["browser_download_url"] =
                "https://evil.example/releases/download/v0.2.0/Vitals-0.2.0-arm64.tar.gz".into()
        });
        // Our host, but a URL whose last segment is not the asset's name.
        let other_name = latest_with(|v| {
            v["assets"][1]["browser_download_url"] =
                "https://github.com/billsun9305/vitals/releases/download/v0.2.0/other.tar.gz".into()
        });
        // Our host, but not under releases/download/.
        let other_path = latest_with(|v| {
            v["assets"][1]["browser_download_url"] =
                "https://github.com/billsun9305/vitals/archive/Vitals-0.2.0-arm64.tar.gz".into()
        });
        for json in [other_host, other_name, other_path] {
            assert!(matches!(
                parse_latest(&json, &Source::github()),
                Err(UpdateError::BadRelease(_))
            ));
        }
    }

    #[test]
    fn not_json_is_a_bad_release() {
        assert!(matches!(
            parse_latest(b"<html>rate limited</html>", &Source::github()),
            Err(UpdateError::BadRelease(_))
        ));
    }

    #[test]
    fn sums_parse_and_look_up_by_name() {
        let sums = "\
0000000000000000000000000000000000000000000000000000000000000000  Vitals-0.2.0-arm64.tar.gz
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff *Vitals-0.2.0.dmg
this line is junk
abc  too-short.txt
";
        assert_eq!(
            parse_sums(sums),
            vec![
                ("0".repeat(64), "Vitals-0.2.0-arm64.tar.gz".to_string()),
                ("f".repeat(64), "Vitals-0.2.0.dmg".to_string()),
            ]
        );
        assert_eq!(expected_sum(sums, "Vitals-0.2.0-arm64.tar.gz"), Some([0u8; 32]));
        assert_eq!(expected_sum(sums, "Vitals-0.2.0.dmg"), Some([0xffu8; 32]));
        assert_eq!(expected_sum(sums, "missing"), None);
        // Upper-case hex is accepted.
        let upper = format!("{}  x\n", "AB".repeat(32));
        assert_eq!(expected_sum(&upper, "x"), Some([0xabu8; 32]));
    }

    #[test]
    fn github_source_points_at_the_repository() {
        let s = Source::github();
        assert_eq!(s.api_latest, "https://api.github.com/repos/billsun9305/vitals/releases/latest");
        assert_eq!(s.download_base, "https://github.com/billsun9305/vitals/releases/download/");
    }

    #[test]
    fn loopback_accepts_only_local_http_bases() {
        let s = Source::loopback("http://127.0.0.1:8000/").unwrap();
        assert_eq!(s.api_latest, "http://127.0.0.1:8000/releases/latest");
        assert_eq!(s.download_base, "http://127.0.0.1:8000/");
        assert!(Source::loopback("http://localhost:9/").is_ok());
        for bad in [
            "https://127.0.0.1:8000/",
            "http://127.0.0.1:8000",
            "http://127.0.0.1/",
            "http://127.0.0.2:8000/",
            "http://example.com:8000/",
            "http://127.0.0.1:8000/sub/",
            "http://127.0.0.1:80x/",
            "",
        ] {
            assert!(
                matches!(Source::loopback(bad), Err(UpdateError::BadSource(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn errors_display_their_reason() {
        assert_eq!(UpdateError::BadChecksum.to_string(), "the download's SHA-256 does not match SHA256SUMS");
        assert_eq!(UpdateError::Http("x: HTTP 404".into()).to_string(), "x: HTTP 404");
        let io: UpdateError = std::io::Error::other("disk full").into();
        assert_eq!(io.to_string(), "disk full");
    }
}
```

- [ ] **Step 5: Run them to see them fail**

```bash
cargo test -p vitals --lib update::release 2>&1 | grep -E "^error|cannot find" | head -5
```

Expected: compile errors — `Version`, `Source`, `parse_latest` … not found.

- [ ] **Step 6: Write the module above the tests**

```rust
//! What a release is, and how the updater reads one: version numbers, the
//! `releases/latest` JSON, the `SHA256SUMS` file, and the one place the
//! download host is allowed to be. No AppKit, no I/O: everything here is a
//! function of its arguments, and unit-tested as such.

use std::fmt;

/// The repository releases come from. `Source::github` and the asset
/// contract in the design doc are both derived from it.
pub const REPO: &str = "billsun9305/vitals";

/// The checksum asset's name.
pub const SUMS_NAME: &str = "SHA256SUMS";

/// A plain `MAJOR.MINOR.PATCH` version.
///
/// Pre-release suffixes are rejected on purpose: `releases/latest` never
/// returns a pre-release, and a bundle's `CFBundleShortVersionString` is
/// always plain, so a suffix anywhere means something is wrong. The derive
/// order of the fields is the comparison order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// `"0.2.0"` or `"v0.2.0"`, surrounding whitespace ignored.
    pub fn parse(s: &str) -> Result<Version, UpdateError> {
        let text = s.trim();
        let body = text.strip_prefix('v').unwrap_or(text);
        let mut parts = body.split('.');
        let mut next = |what: &str| -> Result<u32, UpdateError> {
            parts
                .next()
                .ok_or_else(|| UpdateError::BadVersion(format!("{text:?}: missing {what}")))?
                .parse::<u32>()
                .map_err(|_| UpdateError::BadVersion(format!("{text:?}: {what} is not a number")))
        };
        let major = next("major")?;
        let minor = next("minor")?;
        let patch = next("patch")?;
        if parts.next().is_some() {
            return Err(UpdateError::BadVersion(format!("{text:?}: more than three components")));
        }
        Ok(Version { major, minor, patch })
    }

    /// The version this binary was built as.
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is MAJOR.MINOR.PATCH")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Where releases are looked up and downloaded from.
///
/// `github()` is the only source a shipped build ever uses; `loopback()`
/// exists for the manual end-to-end test and the integration tests, and
/// refuses anything that is not this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The `releases/latest` endpoint.
    pub api_latest: String,
    /// The prefix every accepted asset URL must start with.
    pub download_base: String,
}

impl Source {
    pub fn github() -> Source {
        Source {
            api_latest: format!("https://api.github.com/repos/{REPO}/releases/latest"),
            download_base: format!("https://github.com/{REPO}/releases/download/"),
        }
    }

    /// Accepts exactly `http://127.0.0.1:<port>/` or `http://localhost:<port>/`.
    pub fn loopback(url: &str) -> Result<Source, UpdateError> {
        let bad = || UpdateError::BadSource(url.to_string());
        let rest = url.strip_prefix("http://").ok_or_else(bad)?;
        let (host_port, path) = rest.split_once('/').ok_or_else(bad)?;
        let (host, port) = host_port.split_once(':').ok_or_else(bad)?;
        let local = host == "127.0.0.1" || host == "localhost";
        let numeric = !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit());
        if !local || !numeric || !path.is_empty() {
            return Err(bad());
        }
        Ok(Source {
            api_latest: format!("{url}releases/latest"),
            download_base: url.to_string(),
        })
    }
}

/// One published release, as much of it as the updater needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The release body, Markdown as GitHub stores it.
    pub notes: String,
    /// The release page, for *View Release*.
    pub page_url: String,
    pub archive_url: String,
    pub sums_url: String,
    /// `Vitals-<version>-arm64.tar.gz`, the name looked up in `SHA256SUMS`.
    pub archive_name: String,
}

/// The tarball's name for a version, per the asset contract.
pub fn archive_name(version: Version) -> String {
    format!("Vitals-{version}-arm64.tar.gz")
}

/// Read GitHub's `releases/latest` response.
///
/// Rejects drafts and pre-releases, requires both assets by name, and
/// requires each asset's `browser_download_url` to start with
/// `source.download_base` and end with the asset's own name — a release
/// object is data from the network, and the only URLs it may send us to
/// are ones under our own prefix.
pub fn parse_latest(json: &[u8], source: &Source) -> Result<Release, UpdateError> {
    let v: serde_json::Value = serde_json::from_slice(json)
        .map_err(|e| UpdateError::BadRelease(format!("not JSON: {e}")))?;
    if v["draft"].as_bool() == Some(true) {
        return Err(UpdateError::BadRelease("latest release is a draft".into()));
    }
    if v["prerelease"].as_bool() == Some(true) {
        return Err(UpdateError::BadRelease("latest release is a pre-release".into()));
    }
    let tag = v["tag_name"]
        .as_str()
        .ok_or_else(|| UpdateError::BadRelease("no tag_name".into()))?;
    let version = Version::parse(tag).map_err(|e| UpdateError::BadRelease(format!("tag_name: {e}")))?;
    let assets = v["assets"]
        .as_array()
        .ok_or_else(|| UpdateError::BadRelease("no assets".into()))?;
    let name = archive_name(version);
    let archive_url = asset_url(assets, &name, source)?;
    let sums_url = asset_url(assets, SUMS_NAME, source)?;
    let page_url = v["html_url"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/tag/v{version}"));
    Ok(Release {
        version,
        notes: v["body"].as_str().unwrap_or("").to_string(),
        page_url,
        archive_url,
        sums_url,
        archive_name: name,
    })
}

fn asset_url(assets: &[serde_json::Value], name: &str, source: &Source) -> Result<String, UpdateError> {
    let asset = assets
        .iter()
        .find(|a| a["name"].as_str() == Some(name))
        .ok_or_else(|| UpdateError::BadRelease(format!("release has no asset named {name}")))?;
    let url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| UpdateError::BadRelease(format!("asset {name} has no browser_download_url")))?;
    let under_prefix = url.starts_with(&source.download_base);
    let named_right = url.rsplit('/').next() == Some(name);
    if !under_prefix || !named_right {
        return Err(UpdateError::BadRelease(format!(
            "asset {name} is served from {url}, not from under {}",
            source.download_base
        )));
    }
    Ok(url.to_string())
}

/// Every well-formed line of a `SHA256SUMS` file as `(hex digest, file name)`.
///
/// Accepts `shasum -a 256`'s two-space form and the `*name` binary marker
/// some tools emit; skips anything that is not 64 hex digits followed by
/// a name. Digests come back lower-case.
pub fn parse_sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (hex, rest) = line.trim().split_once(char::is_whitespace)?;
            let name = rest.trim_start().trim_start_matches('*');
            let hex_ok = hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit());
            if !hex_ok || name.is_empty() {
                return None;
            }
            Some((hex.to_ascii_lowercase(), name.to_string()))
        })
        .collect()
}

/// The digest `SHA256SUMS` records for `name`, as bytes.
pub fn expected_sum(sums: &str, name: &str) -> Option<[u8; 32]> {
    let (hex, _) = parse_sums(sums).into_iter().find(|(_, n)| n == name)?;
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// Everything that can go wrong between "is there an update?" and "it is
/// installed". Each variant's string is the reason shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    BadVersion(String),
    BadSource(String),
    BadRelease(String),
    Http(String),
    BadChecksum,
    BadSignature(String),
    Io(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::BadVersion(s) => write!(f, "bad version {s}"),
            UpdateError::BadSource(s) => write!(f, "update source must be http://127.0.0.1:<port>/ or http://localhost:<port>/, not {s:?}"),
            UpdateError::BadRelease(s) => write!(f, "bad release: {s}"),
            UpdateError::Http(s) => write!(f, "{s}"),
            UpdateError::BadChecksum => write!(f, "the download's SHA-256 does not match SHA256SUMS"),
            UpdateError::BadSignature(s) => write!(f, "the update's code signature was rejected: {s}"),
            UpdateError::Io(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<std::io::Error> for UpdateError {
    fn from(e: std::io::Error) -> Self {
        UpdateError::Io(e.to_string())
    }
}
```

- [ ] **Step 7: Run the tests**

```bash
cargo test -p vitals --lib update::release 2>&1 | tail -4
```

Expected: `test result: ok. 14 passed`.

- [ ] **Step 8: Lint, format, commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets --examples -- -D warnings 2>&1 | tail -2
git add Cargo.toml Cargo.lock crates/app/Cargo.toml crates/app/src/lib.rs crates/app/src/update/mod.rs crates/app/src/update/release.rs
git commit -m "feat(update): release model — versions, sources, latest JSON, SHA256SUMS

Pure parsing for the in-app updater: Version (MAJOR.MINOR.PATCH, ordered
numerically, suffixes rejected), Source (GitHub, or a loopback base for
tests), Release from GitHub's releases/latest JSON (drafts, pre-releases,
missing assets and assets served from anywhere but our own
releases/download/ prefix are rejected), SHA256SUMS lookup, and
UpdateError. Adds flate2, tar, sha2 0.10 (0.11 needs Rust 1.85),
objc2-security, objc2-core-foundation and objc2-service-management for
the tasks that follow.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: `update::http` — fetch and download over `NSURLSession`

Section §2 "The request" and §4 step 2 ("`NSURLSession` download tasks, `timeoutIntervalForRequest` 10 s").

**Files:**
- Create: `crates/app/src/update/http.rs`
- Modify: `crates/app/src/update/mod.rs` (add `pub mod http;`)

**Interfaces:**
- Consumes: `UpdateError` (Task 3).
- Produces: `pub fn fetch(url: &str) -> Result<Vec<u8>, UpdateError>`; `pub fn download(url: &str, dir: &Path) -> Result<PathBuf, UpdateError>` (file named after the URL's last path segment); `pub const TIMEOUT_S: f64 = 10.0`. Both block; both are only called from worker threads.

Binding facts (verified in `objc2-foundation 0.3.2`): `NSURLSessionConfiguration::ephemeralSessionConfiguration()`, `setTimeoutIntervalForRequest(f64)`, `NSURLSession::sessionWithConfiguration(&cfg)`, `NSMutableURLRequest::requestWithURL(&NSURL)`, `setValue_forHTTPHeaderField(Option<&NSString>, &NSString)` — all safe. `dataTaskWithRequest_completionHandler(&self, &NSURLRequest, &DynBlock<dyn Fn(*mut NSData, *mut NSURLResponse, *mut NSError)>)` and `downloadTaskWithRequest_completionHandler(&self, &NSURLRequest, &DynBlock<dyn Fn(*mut NSURL, *mut NSURLResponse, *mut NSError)>)` are `unsafe`. `resume()` and `finishTasksAndInvalidate()` are safe. `NSHTTPURLResponse::statusCode() -> isize`; `NSURLResponse` downcasts with `.downcast_ref::<NSHTTPURLResponse>()`. `NSFileManager::moveItemAtURL_toURL_error(&NSURL, &NSURL) -> Result<(), Retained<NSError>>` is safe. `block2::RcBlock<F>` derefs to `DynBlock<F>`, so `&handler` is the argument.

- [ ] **Step 1: Write the failing tests**

Create `crates/app/src/update/http.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tiny_http::{Response, Server};

    /// A loopback server answering fixed routes on a background thread
    /// until dropped, recording every request header it sees.
    struct Fixture {
        base: String,
        server: Arc<Server>,
        thread: Option<std::thread::JoinHandle<()>>,
        headers: Arc<Mutex<Vec<(String, String)>>>,
    }

    fn serve(routes: Vec<(&'static str, u16, &'static [u8])>) -> Fixture {
        let server = Arc::new(Server::http("127.0.0.1:0").expect("bind loopback"));
        let port = server.server_addr().to_ip().expect("ip address").port();
        let headers = Arc::new(Mutex::new(Vec::new()));
        let (s, seen) = (Arc::clone(&server), Arc::clone(&headers));
        let thread = std::thread::spawn(move || {
            for request in s.incoming_requests() {
                for h in request.headers() {
                    seen.lock().unwrap().push((h.field.to_string(), h.value.to_string()));
                }
                let (status, body) = routes
                    .iter()
                    .find(|(path, _, _)| *path == request.url())
                    .map(|(_, status, body)| (*status, body.to_vec()))
                    .unwrap_or((404, b"not found".to_vec()));
                let _ = request.respond(Response::from_data(body).with_status_code(status));
            }
        });
        Fixture { base: format!("http://127.0.0.1:{port}"), server, thread: Some(thread), headers }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.unblock();
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// A port nothing listens on: bind, read the number, close.
    fn closed_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn fetch_returns_the_body_and_sends_exactly_the_github_headers() {
        let fx = serve(vec![("/releases/latest", 200, b"{\"tag_name\":\"v9.9.9\"}")]);
        let body = fetch(&format!("{}/releases/latest", fx.base)).unwrap();
        assert_eq!(body, b"{\"tag_name\":\"v9.9.9\"}");
        let headers = fx.headers.lock().unwrap().clone();
        let find = |name: &str| {
            headers
                .iter()
                .find(|(f, _)| f.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        assert_eq!(find("Accept").as_deref(), Some("application/vnd.github+json"));
        assert_eq!(find("X-GitHub-Api-Version").as_deref(), Some("2022-11-28"));
        assert_eq!(
            find("User-Agent").as_deref(),
            Some(format!("vitals/{}", env!("CARGO_PKG_VERSION")).as_str())
        );
        assert!(find("Authorization").is_none());
        assert!(find("Cookie").is_none());
    }

    #[test]
    fn fetch_reports_404_and_403_by_status() {
        let fx = serve(vec![("/forbidden", 403, b"rate limited")]);
        match fetch(&format!("{}/missing", fx.base)) {
            Err(UpdateError::Http(reason)) => assert!(reason.contains("HTTP 404"), "{reason}"),
            other => panic!("expected Http(404), got {other:?}"),
        }
        match fetch(&format!("{}/forbidden", fx.base)) {
            Err(UpdateError::Http(reason)) => assert!(reason.contains("HTTP 403"), "{reason}"),
            other => panic!("expected Http(403), got {other:?}"),
        }
    }

    #[test]
    fn fetch_reports_a_refused_connection() {
        let url = format!("http://127.0.0.1:{}/releases/latest", closed_port());
        match fetch(&url) {
            Err(UpdateError::Http(reason)) => assert!(reason.contains(&url), "{reason}"),
            other => panic!("expected Http(..), got {other:?}"),
        }
    }

    #[test]
    fn fetch_rejects_a_string_that_is_not_a_url() {
        assert!(matches!(fetch("not a url at all"), Err(UpdateError::Http(_))));
    }

    #[test]
    fn download_writes_the_file_named_after_the_url() {
        let fx = serve(vec![("/Vitals-0.2.0-arm64.tar.gz", 200, b"tarball bytes")]);
        let dir = tempfile::tempdir().unwrap();
        let path = download(&format!("{}/Vitals-0.2.0-arm64.tar.gz", fx.base), dir.path()).unwrap();
        assert_eq!(path, dir.path().join("Vitals-0.2.0-arm64.tar.gz"));
        assert_eq!(std::fs::read(&path).unwrap(), b"tarball bytes");
    }

    #[test]
    fn download_of_a_404_leaves_no_file() {
        let fx = serve(vec![]);
        let dir = tempfile::tempdir().unwrap();
        let result = download(&format!("{}/SHA256SUMS", fx.base), dir.path());
        assert!(matches!(result, Err(UpdateError::Http(_))), "{result:?}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
```

- [ ] **Step 2: Run them to see them fail**

```bash
cargo test -p vitals --lib update::http 2>&1 | grep -E "^error" | head -3
```

Expected: compile errors, `fetch` and `download` not found.

- [ ] **Step 3: Write the module above the tests**

```rust
//! HTTP over `NSURLSession`: one function that fetches a small body into
//! memory and one that downloads to a file.
//!
//! Both block the calling thread — they are only ever called from the
//! checker's and the installer's worker threads — and both build a fresh
//! ephemeral session per call and invalidate it afterwards, so nothing
//! lingers between checks (the idle budget in `docs/budget.md`).
//!
//! The completion block runs on a queue of the session's choosing, never
//! on the caller's thread, so it must not hand Objective-C objects back
//! across: it copies what it needs into plain Rust values and sends them
//! down an `mpsc` channel, the same rule `tray/child.rs` follows for the
//! launch completion handler.

use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::mpsc;

use objc2::rc::Retained;
use objc2_foundation::{
    NSData, NSError, NSFileManager, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURL,
    NSURLResponse, NSURLSession, NSURLSessionConfiguration,
};

use super::release::UpdateError;

/// Seconds a request may wait for the server between bytes. This is
/// `timeoutIntervalForRequest`, not a cap on a transfer in progress, so a
/// slow download of the tarball is not cut off half way.
pub const TIMEOUT_S: f64 = 10.0;

/// The request every call sends: the GitHub REST headers and our own
/// `User-Agent`. Nothing else — no token, no cookie.
fn request(url: &str) -> Result<Retained<NSMutableURLRequest>, UpdateError> {
    let ns_url = NSURL::URLWithString(&NSString::from_str(url))
        .ok_or_else(|| UpdateError::Http(format!("not a URL: {url}")))?;
    let req = NSMutableURLRequest::requestWithURL(&ns_url);
    let user_agent = format!("vitals/{}", env!("CARGO_PKG_VERSION"));
    for (field, value) in [
        ("Accept", "application/vnd.github+json"),
        ("X-GitHub-Api-Version", "2022-11-28"),
        ("User-Agent", user_agent.as_str()),
    ] {
        req.setValue_forHTTPHeaderField(Some(&NSString::from_str(value)), &NSString::from_str(field));
    }
    Ok(req)
}

/// One session per call: ephemeral (no cache, no cookie jar, nothing on
/// disk) with the request timeout above. The caller invalidates it.
fn session() -> Retained<NSURLSession> {
    let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
    config.setTimeoutIntervalForRequest(TIMEOUT_S);
    NSURLSession::sessionWithConfiguration(&config)
}

/// Turn the `(response, error)` pair every completion handler receives
/// into `Ok(())` for a 2xx or the reason it was not one.
///
/// # Safety
/// `response` and `error` must be the pointers a completion handler was
/// given, valid for the duration of the call.
unsafe fn status_of(response: *mut NSURLResponse, error: *mut NSError) -> Result<(), String> {
    if let Some(error) = NonNull::new(error) {
        // SAFETY: per this function's contract.
        return Err(unsafe { error.as_ref() }.localizedDescription().to_string());
    }
    let Some(response) = NonNull::new(response) else {
        return Err("no response".to_string());
    };
    // SAFETY: per this function's contract.
    let response = unsafe { response.as_ref() };
    match response.downcast_ref::<NSHTTPURLResponse>() {
        Some(http) => {
            let code = http.statusCode();
            if (200..300).contains(&code) {
                Ok(())
            } else {
                Err(format!("HTTP {code}"))
            }
        }
        None => Err("not an HTTP response".to_string()),
    }
}

/// `GET url`, the whole body in memory. For the `releases/latest` JSON.
pub fn fetch(url: &str) -> Result<Vec<u8>, UpdateError> {
    let req = request(url)?;
    let session = session();
    let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>();
    let handler = block2::RcBlock::new(
        move |data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
            // SAFETY: the three pointers are what NSURLSession hands its
            // completion handler, valid for this call.
            let result = unsafe { status_of(response, error) }.and_then(|()| {
                NonNull::new(data)
                    // SAFETY: as above.
                    .map(|d| unsafe { d.as_ref() }.to_vec())
                    .ok_or_else(|| "empty body".to_string())
            });
            let _ = tx.send(result);
        },
    );
    // SAFETY: `req` is a complete request, and the session retains the
    // block until it has run, so it outlives the task.
    let task = unsafe { session.dataTaskWithRequest_completionHandler(&req, &handler) };
    task.resume();
    let result = rx
        .recv()
        .map_err(|_| UpdateError::Http(format!("{url}: request abandoned")))?;
    session.finishTasksAndInvalidate();
    result.map_err(|reason| UpdateError::Http(format!("{url}: {reason}")))
}

/// `GET url` to a file in `dir`, named after the URL's last path segment.
/// For `SHA256SUMS` and the tarball.
pub fn download(url: &str, dir: &Path) -> Result<PathBuf, UpdateError> {
    let name = url
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty() && !n.contains('?'))
        .ok_or_else(|| UpdateError::Http(format!("no file name at the end of {url}")))?;
    let dest = dir.join(name);
    let req = request(url)?;
    let session = session();
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    // The block runs on another thread, so it gets a `PathBuf` (Send) and
    // builds its own `NSURL` there, rather than capturing one from here.
    let dest_for_block = dest.clone();
    let handler = block2::RcBlock::new(
        move |location: *mut NSURL, response: *mut NSURLResponse, error: *mut NSError| {
            // SAFETY: the three pointers are what NSURLSession hands its
            // completion handler, valid for this call.
            let result = unsafe { status_of(response, error) }.and_then(|()| {
                let location = NonNull::new(location).ok_or_else(|| "no file".to_string())?;
                // The temporary file is deleted when this block returns,
                // so it is moved now, on this thread.
                let dest = NSURL::fileURLWithPath(&NSString::from_str(&dest_for_block.to_string_lossy()));
                NSFileManager::defaultManager()
                    // SAFETY: as above.
                    .moveItemAtURL_toURL_error(unsafe { location.as_ref() }, &dest)
                    .map_err(|e| e.localizedDescription().to_string())
            });
            let _ = tx.send(result);
        },
    );
    // SAFETY: as in `fetch`.
    let task = unsafe { session.downloadTaskWithRequest_completionHandler(&req, &handler) };
    task.resume();
    let result = rx
        .recv()
        .map_err(|_| UpdateError::Http(format!("{url}: download abandoned")))?;
    session.finishTasksAndInvalidate();
    result
        .map(|()| dest)
        .map_err(|reason| UpdateError::Http(format!("{url}: {reason}")))
}
```

Add `pub mod http;` to `crates/app/src/update/mod.rs`, keeping the list alphabetical.

- [ ] **Step 4: Run the tests**

```bash
cargo test -p vitals --lib update::http -- --nocapture 2>&1 | tail -4
```

Expected: `test result: ok. 6 passed`. If `download_of_a_404_leaves_no_file` fails because a file exists, `status_of` is not being consulted before the move — the 404 body must never be moved into `dir`.

- [ ] **Step 5: Lint, format, commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets --examples -- -D warnings 2>&1 | tail -2
git add crates/app/src/update/http.rs crates/app/src/update/mod.rs
git commit -m "feat(update): fetch and download over NSURLSession

Two blocking calls for the updater's worker threads. Each builds an
ephemeral session, sends exactly the GitHub REST headers plus
User-Agent: vitals/<version>, and invalidates the session afterwards.
The completion block copies the body (or moves the downloaded file) on
its own queue and reports through a channel; no Objective-C object
crosses a thread. Tested against a tiny_http fixture: headers sent,
404/403/refused reported as Http(..), and a 404 download leaves no file.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: `update::codesign` — the Team ID question

Section §4 step 4, "Then check the signature", and §4 "Security".

**Files:**
- Create: `crates/app/src/update/codesign.rs`
- Modify: `crates/app/src/update/mod.rs` (add `pub mod codesign;`)

**Interfaces:**
- Produces: `pub fn team_id_of_self() -> Option<String>`; `pub fn verify_team(bundle: &Path, team_id: &str) -> Result<(), String>`; `pub fn requirement_for(team_id: &str) -> String`.

Binding facts (verified in `objc2-security 0.3.2` / `objc2-core-foundation 0.3.2`): all Security calls are `unsafe fn … -> OSStatus` writing through a `NonNull` out-pointer — `SecCode::copy_self(SecCSFlags, NonNull<*mut SecCode>)`, `SecCode::copy_static_code(&self, SecCSFlags, NonNull<*const SecStaticCode>)`, `SecCode::copy_signing_information(&SecStaticCode, SecCSFlags, NonNull<*const CFDictionary>)` (an associated function taking the static code as its first argument), `SecStaticCode::create_with_path(&CFURL, SecCSFlags, NonNull<*const SecStaticCode>)`, `SecRequirement::create_with_string(&CFString, SecCSFlags, NonNull<*mut SecRequirement>)`, `SecStaticCode::check_validity(&self, SecCSFlags, Option<&SecRequirement>)`. `SecCSFlags(pub u32)` is a bitflags struct with `SecCSFlags::DefaultFlags` (0); `kSecCSSigningInformation: u32 = 2` is a plain `const`; `kSecCodeInfoTeamIdentifier: &'static CFString` is an `extern static` (reading it is `unsafe`). `CFRetained::from_raw(NonNull<T>)` takes ownership of a +1 reference. `CFDictionary::value(&self, *const c_void) -> *const c_void` is `unsafe`. `CFType::downcast_ref::<CFString>()`; `CFString` implements `Display`. `CFURL::from_file_path(impl AsRef<Path>) -> Option<CFRetained<CFURL>>`.

- [ ] **Step 1: Write the failing tests**

Create `crates/app/src/update/codesign.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_requirement_is_the_developer_id_designated_form() {
        assert_eq!(
            requirement_for("4WD8D5Y6NF"),
            "anchor apple generic and certificate leaf[subject.OU] = \"4WD8D5Y6NF\""
        );
    }

    #[test]
    fn a_test_binary_has_no_team_id() {
        // cargo's test binaries are ad-hoc signed by the linker: a valid
        // signature with no certificate chain and therefore no Team ID.
        // This is also the case the installer treats as "skip the check".
        assert_eq!(team_id_of_self(), None);
    }

    #[test]
    fn a_missing_bundle_fails_to_verify() {
        let err = verify_team(std::path::Path::new("/nonexistent/Vitals.app"), "4WD8D5Y6NF").unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn an_ad_hoc_binary_does_not_satisfy_a_team_requirement() {
        let me = std::env::current_exe().unwrap();
        let err = verify_team(&me, "4WD8D5Y6NF").unwrap_err();
        assert!(err.contains("OSStatus"), "{err}");
    }
}
```

- [ ] **Step 2: Run them to see them fail**

```bash
cargo test -p vitals --lib update::codesign 2>&1 | grep -E "^error" | head -3
```

Expected: compile errors, the three functions not found.

- [ ] **Step 3: Write the module above the tests**

```rust
//! Two questions for the Security framework: which Team ID signed the
//! running bundle, and does a staged bundle carry a valid Developer ID
//! signature from that same team.
//!
//! This is what makes an update authentic rather than merely intact. The
//! SHA-256 in `SHA256SUMS` proves the tarball is the one the release
//! published; only the signature proves the release came from the person
//! holding the certificate, which a compromised GitHub account does not.
//! An ad-hoc copy (a local build, or a release built without the secrets)
//! has no Team ID, and the installer skips the check for it — `SECURITY.md`
//! says so.

use std::ffi::c_void;
use std::path::Path;
use std::ptr::NonNull;

use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType, CFURL};
use objc2_security::{
    kSecCSSigningInformation, kSecCodeInfoTeamIdentifier, SecCSFlags, SecCode, SecRequirement,
    SecStaticCode,
};

/// The designated requirement an update must satisfy: signed through
/// Apple's Developer ID chain (`anchor apple generic`) by a leaf
/// certificate whose organisational unit is this team.
pub fn requirement_for(team_id: &str) -> String {
    format!("anchor apple generic and certificate leaf[subject.OU] = \"{team_id}\"")
}

/// The Team ID in the running process's own code signature, or `None`
/// when there is none — an ad-hoc signature, or no signature at all.
pub fn team_id_of_self() -> Option<String> {
    let mut code: *mut SecCode = std::ptr::null_mut();
    // SAFETY: `code` is a valid out-slot for one pointer. The framework
    // writes it only on success, which is checked before it is read.
    let status = unsafe { SecCode::copy_self(SecCSFlags::DefaultFlags, NonNull::from(&mut code)) };
    if status != 0 {
        return None;
    }
    // SAFETY: on success the slot holds a +1 reference this scope now owns.
    let code = unsafe { CFRetained::from_raw(NonNull::new(code)?) };

    let mut static_code: *const SecStaticCode = std::ptr::null();
    // SAFETY: as above.
    let status = unsafe {
        code.copy_static_code(SecCSFlags::DefaultFlags, NonNull::from(&mut static_code))
    };
    if status != 0 {
        return None;
    }
    // SAFETY: as above; `cast_mut` changes only the pointer's type.
    let static_code = unsafe { CFRetained::from_raw(NonNull::new(static_code.cast_mut())?) };
    team_id_of(&static_code)
}

/// The `teamid` entry of a code object's signing information.
fn team_id_of(code: &SecStaticCode) -> Option<String> {
    let mut info: *const CFDictionary = std::ptr::null();
    // SAFETY: `info` is a valid out-slot; the signing-information flag asks
    // for the certificate-derived entries, which is where the Team ID is.
    let status = unsafe {
        SecCode::copy_signing_information(
            code,
            SecCSFlags(kSecCSSigningInformation),
            NonNull::from(&mut info),
        )
    };
    if status != 0 {
        return None;
    }
    // SAFETY: on success the slot holds a +1 reference this scope now owns.
    let info = unsafe { CFRetained::from_raw(NonNull::new(info.cast_mut())?) };
    // SAFETY: the key is the framework's own constant, and the dictionary's
    // keys are CFStrings, so `CFDictionaryGetValue` with it is the
    // documented lookup. The value is borrowed from `info`, alive until
    // this function returns.
    let value = unsafe {
        let key: &CFString = kSecCodeInfoTeamIdentifier;
        info.value((key as *const CFString).cast::<c_void>())
    };
    let value = NonNull::new(value.cast_mut())?;
    // SAFETY: a non-null value in this dictionary is a CF object, and
    // `downcast_ref` checks that it is a CFString rather than assuming.
    let team = unsafe { value.cast::<CFType>().as_ref() }.downcast_ref::<CFString>()?;
    Some(team.to_string())
}

/// Does the bundle at `bundle` carry a valid signature satisfying
/// `requirement_for(team_id)`? `Err` carries the reason, `OSStatus`
/// included, for the failure alert.
pub fn verify_team(bundle: &Path, team_id: &str) -> Result<(), String> {
    let url = CFURL::from_file_path(bundle)
        .ok_or_else(|| format!("not a file path: {}", bundle.display()))?;

    let mut code: *const SecStaticCode = std::ptr::null();
    // SAFETY: `url` is a live CFURL and `code` a valid out-slot; checked
    // before read.
    let status = unsafe {
        SecStaticCode::create_with_path(&url, SecCSFlags::DefaultFlags, NonNull::from(&mut code))
    };
    if status != 0 {
        return Err(format!("could not read the code signature of {}: OSStatus {status}", bundle.display()));
    }
    // SAFETY: on success the slot holds a +1 reference this scope now owns.
    let code = unsafe { CFRetained::from_raw(NonNull::new(code.cast_mut()).ok_or("no code object")?) };

    let text = CFString::from_str(&requirement_for(team_id));
    let mut requirement: *mut SecRequirement = std::ptr::null_mut();
    // SAFETY: `text` is a live CFString and `requirement` a valid out-slot;
    // checked before read.
    let status = unsafe {
        SecRequirement::create_with_string(&text, SecCSFlags::DefaultFlags, NonNull::from(&mut requirement))
    };
    if status != 0 {
        return Err(format!("could not compile the code requirement: OSStatus {status}"));
    }
    // SAFETY: as above.
    let requirement = unsafe { CFRetained::from_raw(NonNull::new(requirement).ok_or("no requirement object")?) };

    // SAFETY: both objects are alive for the call.
    let status = unsafe { code.check_validity(SecCSFlags::DefaultFlags, Some(&requirement)) };
    if status == 0 {
        Ok(())
    } else {
        Err(format!(
            "{} does not satisfy `{}`: OSStatus {status}",
            bundle.display(),
            requirement_for(team_id)
        ))
    }
}
```

Add `pub mod codesign;` to `update/mod.rs`.

- [ ] **Step 4: Run the tests**

```bash
cargo test -p vitals --lib update::codesign 2>&1 | tail -4
```

Expected: `test result: ok. 4 passed`. If `a_test_binary_has_no_team_id` fails with `Some(..)`, the machine's `cargo` is signing test binaries with a real identity — report it; do not change the test.

- [ ] **Step 5: Confirm the requirement string against `codesign` itself**

```bash
codesign -d -r- /System/Applications/Calculator.app 2>&1 | head -3
```

Expected: Apple's own designated requirement uses the same `anchor apple` / `certificate leaf[subject.OU]` vocabulary (Apple's apps use `anchor apple`, third-party ones `anchor apple generic`), which confirms the grammar `requirement_for` emits. Nothing to change; this is a sanity check on the string.

- [ ] **Step 6: Lint, format, commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets --examples -- -D warnings 2>&1 | tail -2
git add crates/app/src/update/codesign.rs crates/app/src/update/mod.rs
git commit -m "feat(update): Team ID of the running copy, and a signature check against it

team_id_of_self reads the teamid entry of our own signing information;
verify_team compiles the designated requirement for that team and asks
SecStaticCodeCheckValidity whether a staged bundle satisfies it. Through
objc2-security's method-style names, since the free functions are
deprecated and -D warnings would refuse them. An ad-hoc binary has no
Team ID and does not satisfy any team's requirement; both are tested on
the test binary itself.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: `update::install` — stage, download, verify, unpack, swap

Section §4 "Install" steps 1–6, "Failure" (the staging directory is removed), "Security" (`set_overwrite(false)`, fresh directory) and §5 "Integration — `crates/app/tests/update.rs`" cases 1–5.

**Files:**
- Create: `crates/app/src/update/install.rs`
- Modify: `crates/app/src/update/mod.rs` (add `pub mod install;`)
- Create: `crates/app/tests/update.rs`

**Interfaces:**
- Consumes: `http::download`, `codesign::{team_id_of_self, verify_team}`, `release::{expected_sum, Release, Source, UpdateError, Version}`.
- Produces:
  - `pub const BUNDLE_NAME: &str = "Vitals.app"`.
  - `pub fn prepare(source: &Source, release: &Release, app_url: &Path) -> Result<Staged, UpdateError>` — steps 1–4 with the staging directory from `NSItemReplacementDirectory`.
  - `pub fn prepare_in(dir: PathBuf, source: &Source, release: &Release) -> Result<Staged, UpdateError>` — the same with a caller-chosen, existing, empty directory; removed on failure.
  - `pub struct Staged` with `fn dir(&self) -> &Path`, `fn app(&self) -> &Path`, `fn signature_checked(&self) -> bool`, `fn swap_into(self, app_url: &Path) -> Result<(), UpdateError>`; `Drop` removes `dir` unless `swap_into` ran.
  - `pub struct Helper` with `fn spawn(parent: u32, app_url: &Path) -> Result<Helper, UpdateError>` and `fn cancel(self)` (SIGTERM, then reap).
  - `pub fn run(source: &Source, release: &Release, app_url: &Path) -> Result<(), UpdateError>` — prepare, spawn, swap; cancels the helper if the swap fails. Used by Task 10.

Binding facts (verified): `NSFileManager::URLForDirectory_inDomain_appropriateForURL_create_error(NSSearchPathDirectory::ItemReplacementDirectory, NSSearchPathDomainMask::UserDomainMask, Some(&url), true) -> Result<Retained<NSURL>, Retained<NSError>>` (safe); `replaceItemAtURL_withItemAtURL_backupItemName_options_resultingItemURL_error(&NSURL, &NSURL, Option<&NSString>, NSFileManagerItemReplacementOptions, Option<&mut Option<Retained<NSURL>>>) -> Result<(), Retained<NSError>>` (safe; `NSFileManagerItemReplacementOptions::empty()`); `NSDictionary::<NSString, AnyObject>::dictionaryWithContentsOfURL(&NSURL) -> Option<Retained<…>>` (unsafe); `objectForKey(&NSString) -> Option<Retained<AnyObject>>`; `NSURL::path() -> Option<Retained<NSString>>`. `tar::Archive::unpack` applies each entry's mode masked to `0o777` even with the default `preserve_permissions = false`, so executable bits survive; `set_unpack_xattrs` defaults to `false`, so no extended attribute is written.

- [ ] **Step 1: Write the failing integration tests**

Create `crates/app/tests/update.rs`:

```rust
//! The installer end to end, against a loopback release server.
//!
//! No process is launched: `Helper::spawn` is never called, and
//! `swap_into` replaces a temporary "installed" bundle under
//! `tempfile::tempdir()`, never a real one. The fixture builds the tarball
//! itself from a bundle whose Info.plist carries the target version.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use vitals::update::install::{prepare, prepare_in};
use vitals::update::release::{archive_name, parse_latest, Release, Source, UpdateError, Version};

const OLD: &str = "0.1.0";
const NEW: &str = "0.1.1";

/// A minimal bundle: an Info.plist with the version, and an executable.
fn make_bundle(dir: &Path, version: &str) -> PathBuf {
    let app = dir.join("Vitals.app");
    fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    fs::write(
        app.join("Contents/Info.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\"><dict>\n\
             <key>CFBundleIdentifier</key><string>com.billsun.vitals</string>\n\
             <key>CFBundleShortVersionString</key><string>{version}</string>\n\
             </dict></plist>\n"
        ),
    )
    .unwrap();
    let exe = app.join("Contents/MacOS/vitals");
    fs::write(&exe, format!("#!/bin/sh\necho vitals {version}\n")).unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    app
}

/// `app` as a gzipped tar whose one top-level entry is `Vitals.app/`,
/// plus an optional second top-level file for the "two entries" case.
fn tarball(app: &Path, extra_top_level: Option<&str>) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    {
        let mut tar = tar::Builder::new(&mut gz);
        tar.append_dir_all("Vitals.app", app).unwrap();
        if let Some(name) = extra_top_level {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, std::io::empty()).unwrap();
        }
        tar.finish().unwrap();
    }
    gz.finish().unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn plist_version(app: &Path) -> String {
    let text = fs::read_to_string(app.join("Contents/Info.plist")).unwrap();
    text.split("<key>CFBundleShortVersionString</key><string>")
        .nth(1)
        .unwrap()
        .split("</string>")
        .next()
        .unwrap()
        .to_string()
}

fn has_quarantine(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: both strings are valid and NUL-terminated; a null buffer of
    // size 0 only asks whether the attribute exists.
    let n = unsafe {
        libc::getxattr(
            path.as_ptr(),
            c"com.apple.quarantine".as_ptr(),
            std::ptr::null_mut(),
            0,
            0,
            0,
        )
    };
    n >= 0
}

/// Every file under `dir` with its bytes, sorted, for "unchanged" checks.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// What the fixture serves. Statuses are 200 unless a test overrides one.
struct Files {
    latest: Vec<u8>,
    sums: Vec<u8>,
    sums_status: u16,
    archive: Vec<u8>,
    archive_name: String,
    archive_status: u16,
}

impl Files {
    /// A self-consistent release of `version` built from `archive`: the
    /// asset is named for the version and `SHA256SUMS` carries its digest.
    fn consistent(base: &str, version: &str, archive: Vec<u8>) -> Files {
        let name = archive_name(Version::parse(version).unwrap());
        let sums = format!("{}  {name}\n", sha256_hex(&archive));
        Files {
            latest: latest_json(base, version, &name),
            sums: sums.into_bytes(),
            sums_status: 200,
            archive,
            archive_name: name,
            archive_status: 200,
        }
    }
}

fn latest_json(base: &str, version: &str, archive_name: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "tag_name": format!("v{version}"),
        "draft": false,
        "prerelease": false,
        "html_url": format!("{base}releases/tag/v{version}"),
        "body": "Test release.",
        "assets": [
            {"name": archive_name, "browser_download_url": format!("{base}{archive_name}")},
            {"name": "SHA256SUMS", "browser_download_url": format!("{base}SHA256SUMS")}
        ]
    }))
    .unwrap()
}

/// A loopback release server. `start` binds so the base URL is known
/// before the files (which embed it) are built; `serve` starts answering.
struct Fixture {
    base: String,
    server: Arc<tiny_http::Server>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    fn start() -> Fixture {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        Fixture { base: format!("http://127.0.0.1:{port}/"), server, thread: None }
    }

    fn serve(&mut self, files: Files) {
        let server = Arc::clone(&self.server);
        self.thread = Some(std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let url = request.url().to_string();
                let (status, body) = if url == "/releases/latest" {
                    (200, files.latest.clone())
                } else if url == "/SHA256SUMS" {
                    (files.sums_status, files.sums.clone())
                } else if url == format!("/{}", files.archive_name) {
                    (files.archive_status, files.archive.clone())
                } else {
                    (404, b"not found".to_vec())
                };
                let _ = request.respond(tiny_http::Response::from_data(body).with_status_code(status));
            }
        }));
    }

    fn source(&self) -> Source {
        Source::loopback(&self.base).unwrap()
    }

    fn release(&self) -> Release {
        let source = self.source();
        let json = vitals::update::http::fetch(&source.api_latest).unwrap();
        parse_latest(&json, &source).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Run `prepare_in` against `files` and require it to fail: the error is
/// returned, and the staging directory and the installed bundle are
/// asserted gone and unchanged respectively.
fn expect_failure(files: Files, fx: &mut Fixture, installed: &Path, tmp: &Path) -> UpdateError {
    fx.serve(files);
    let (source, release) = (fx.source(), fx.release());
    expect_failure_for(&source, &release, installed, tmp)
}

fn expect_failure_for(source: &Source, release: &Release, installed: &Path, tmp: &Path) -> UpdateError {
    let staging = tmp.join("staging");
    fs::create_dir_all(&staging).unwrap();
    let before = snapshot(installed);
    let err = prepare_in(staging.clone(), source, release).err().expect("prepare must fail");
    assert!(!staging.exists(), "staging directory left behind after {err}");
    assert_eq!(snapshot(installed), before, "installed bundle changed after {err}");
    assert_eq!(plist_version(installed), OLD);
    err
}

#[test]
fn a_good_release_replaces_the_installed_bundle() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = make_bundle(&tmp.path().join("installed"), OLD);
    let new_app = make_bundle(&tmp.path().join("new"), NEW);
    let mut fx = Fixture::start();
    let base = fx.base.clone();
    fx.serve(Files::consistent(&base, NEW, tarball(&new_app, None)));
    let (source, release) = (fx.source(), fx.release());
    assert_eq!(release.version, Version::parse(NEW).unwrap());

    let staged = prepare(&source, &release, &installed).unwrap();
    assert!(
        !staged.signature_checked(),
        "the test binary has no Team ID, so the signature check must be skipped"
    );
    let staging_dir = staged.dir().to_path_buf();
    assert!(staging_dir.exists());
    assert_eq!(plist_version(staged.app()), NEW);

    staged.swap_into(&installed).unwrap();

    assert!(!staging_dir.exists(), "staging directory should be gone after the swap");
    assert_eq!(plist_version(&installed), NEW);
    let exe = installed.join("Contents/MacOS/vitals");
    assert_eq!(fs::read_to_string(&exe).unwrap(), format!("#!/bin/sh\necho vitals {NEW}\n"));
    assert_ne!(fs::metadata(&exe).unwrap().permissions().mode() & 0o111, 0, "executable bit lost");
    assert!(!has_quarantine(&exe), "quarantine flag on the new executable");
    assert!(!has_quarantine(&installed), "quarantine flag on the new bundle");
}

#[test]
fn a_wrong_hash_is_rejected_and_nothing_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = make_bundle(&tmp.path().join("installed"), OLD);
    let new_app = make_bundle(&tmp.path().join("new"), NEW);
    let mut fx = Fixture::start();
    let base = fx.base.clone();
    let mut files = Files::consistent(&base, NEW, tarball(&new_app, None));
    files.sums = format!("{}  {}\n", "0".repeat(64), files.archive_name).into_bytes();
    let err = expect_failure(files, &mut fx, &installed, tmp.path());
    assert_eq!(err, UpdateError::BadChecksum);
}

#[test]
fn a_tarball_carrying_the_wrong_version_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = make_bundle(&tmp.path().join("installed"), OLD);
    // The release says 0.1.1, the bundle inside says 0.1.0; the hash matches.
    let stale_app = make_bundle(&tmp.path().join("stale"), OLD);
    let mut fx = Fixture::start();
    let base = fx.base.clone();
    let files = Files::consistent(&base, NEW, tarball(&stale_app, None));
    let err = expect_failure(files, &mut fx, &installed, tmp.path());
    assert!(matches!(err, UpdateError::BadRelease(_)), "{err}");
}

#[test]
fn a_tarball_with_two_top_level_entries_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = make_bundle(&tmp.path().join("installed"), OLD);
    let new_app = make_bundle(&tmp.path().join("new"), NEW);
    let mut fx = Fixture::start();
    let base = fx.base.clone();
    let files = Files::consistent(&base, NEW, tarball(&new_app, Some("README.txt")));
    let err = expect_failure(files, &mut fx, &installed, tmp.path());
    assert!(matches!(err, UpdateError::BadRelease(_)), "{err}");
}

#[test]
fn server_errors_are_reported_and_nothing_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = make_bundle(&tmp.path().join("installed"), OLD);
    let new_app = make_bundle(&tmp.path().join("new"), NEW);

    // SHA256SUMS is a 404.
    {
        let mut fx = Fixture::start();
        let base = fx.base.clone();
        let mut files = Files::consistent(&base, NEW, tarball(&new_app, None));
        files.sums_status = 404;
        let err = expect_failure(files, &mut fx, &installed, tmp.path());
        assert!(matches!(&err, UpdateError::Http(r) if r.contains("404")), "{err}");
    }
    // The tarball is a 403.
    {
        let mut fx = Fixture::start();
        let base = fx.base.clone();
        let mut files = Files::consistent(&base, NEW, tarball(&new_app, None));
        files.archive_status = 403;
        let err = expect_failure(files, &mut fx, &installed, tmp.path());
        assert!(matches!(&err, UpdateError::Http(r) if r.contains("403")), "{err}");
    }
    // Nothing listens at all.
    {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}/");
        let source = Source::loopback(&base).unwrap();
        let name = archive_name(Version::parse(NEW).unwrap());
        let release = Release {
            version: Version::parse(NEW).unwrap(),
            notes: String::new(),
            page_url: base.clone(),
            archive_url: format!("{base}{name}"),
            sums_url: format!("{base}SHA256SUMS"),
            archive_name: name,
        };
        let err = expect_failure_for(&source, &release, &installed, tmp.path());
        assert!(matches!(err, UpdateError::Http(_)), "{err}");
    }
}
```

- [ ] **Step 2: Run them to see them fail**

```bash
cargo test -p vitals --test update 2>&1 | grep -E "^error" | head -3
```

Expected: compile errors, `vitals::update::install` not found.

- [ ] **Step 3: Write `crates/app/src/update/install.rs`**

```rust
//! Staging, downloading, verifying and unpacking an update, and swapping
//! it into place. Everything here runs on the installer's worker thread;
//! nothing touches AppKit.
//!
//! The order is the design's: stage next to the bundle (same volume, so
//! the final swap is a rename), download `SHA256SUMS` then the tarball,
//! check the hash, unpack, check the version inside, check the signature,
//! spawn the relaunch helper, swap. The first failure stops the run and
//! `Staged`'s `Drop` removes the staging directory, so the installed bundle
//! is untouched by anything but a successful `swap_into`.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSDictionary, NSFileManager, NSFileManagerItemReplacementOptions, NSSearchPathDirectory,
    NSSearchPathDomainMask, NSString, NSURL,
};
use sha2::{Digest, Sha256};

use super::codesign;
use super::http;
use super::release::{expected_sum, Release, Source, UpdateError, Version};

/// The tarball's one top-level entry, and the name the bundle keeps.
pub const BUNDLE_NAME: &str = "Vitals.app";

/// A downloaded, verified, unpacked bundle waiting in a staging directory.
///
/// Dropping it without calling `swap_into` deletes the directory.
pub struct Staged {
    dir: PathBuf,
    app: PathBuf,
    signature_checked: bool,
    swapped: bool,
}

impl Staged {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The unpacked `Vitals.app` inside the staging directory.
    pub fn app(&self) -> &Path {
        &self.app
    }

    /// Whether step 4 verified the signature, or skipped it because the
    /// running copy has no Team ID.
    pub fn signature_checked(&self) -> bool {
        self.signature_checked
    }

    /// Step 6: replace the bundle at `app_url` with the staged one, then
    /// remove the staging directory. The running process keeps its mapped
    /// binary; only the path changes owner.
    pub fn swap_into(mut self, app_url: &Path) -> Result<(), UpdateError> {
        let original = file_url(app_url);
        let replacement = file_url(&self.app);
        NSFileManager::defaultManager()
            .replaceItemAtURL_withItemAtURL_backupItemName_options_resultingItemURL_error(
                &original,
                &replacement,
                None,
                NSFileManagerItemReplacementOptions::empty(),
                None,
            )
            .map_err(|e| {
                UpdateError::Io(format!("replacing {}: {}", app_url.display(), e.localizedDescription()))
            })?;
        self.swapped = true;
        let _ = std::fs::remove_dir_all(&self.dir);
        Ok(())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.swapped {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// Steps 1–4: stage next to `app_url`, download, verify, unpack, check.
pub fn prepare(source: &Source, release: &Release, app_url: &Path) -> Result<Staged, UpdateError> {
    prepare_in(staging_dir(app_url)?, source, release)
}

/// `prepare` with a caller-chosen staging directory, which must exist and
/// be empty. It is removed on failure. Tests use this to know where the
/// staging directory was after a failure that returned no `Staged`.
pub fn prepare_in(dir: PathBuf, source: &Source, release: &Release) -> Result<Staged, UpdateError> {
    if !release.archive_url.starts_with(&source.download_base) {
        return Err(UpdateError::BadRelease(format!(
            "{} is not under {}",
            release.archive_url, source.download_base
        )));
    }
    let mut staged = Staged {
        app: dir.join(BUNDLE_NAME),
        dir,
        signature_checked: false,
        swapped: false,
    };

    // Step 2: download. The sums first, so a bad tarball is never kept.
    let sums_path = http::download(&release.sums_url, &staged.dir)?;
    let archive_path = http::download(&release.archive_url, &staged.dir)?;

    // Step 3: verify.
    let sums = std::fs::read_to_string(&sums_path)?;
    let expected = expected_sum(&sums, &release.archive_name).ok_or_else(|| {
        UpdateError::BadRelease(format!("SHA256SUMS has no entry for {}", release.archive_name))
    })?;
    if sha256_of(&archive_path)? != expected {
        return Err(UpdateError::BadChecksum);
    }

    // Step 4: unpack, then the version inside, then the signature.
    unpack(&archive_path, &staged.dir)?;
    let found = bundle_version(&staged.app)?;
    if found != release.version {
        return Err(UpdateError::BadRelease(format!(
            "the archive contains Vitals {found}, not {}",
            release.version
        )));
    }
    match codesign::team_id_of_self() {
        Some(team) => {
            codesign::verify_team(&staged.app, &team).map_err(UpdateError::BadSignature)?;
            staged.signature_checked = true;
        }
        None => eprintln!(
            "vitals: the running copy has no Team ID (ad-hoc signature); skipping the signature check on Vitals {}",
            release.version
        ),
    }
    Ok(staged)
}

/// The whole install: prepare, spawn the helper, swap. The helper is
/// cancelled if the swap fails, so nothing relaunches an unchanged bundle.
/// The caller terminates the app afterwards, on the main thread.
pub fn run(source: &Source, release: &Release, app_url: &Path) -> Result<(), UpdateError> {
    let staged = prepare(source, release, app_url)?;
    let helper = Helper::spawn(std::process::id(), app_url)?;
    match staged.swap_into(app_url) {
        Ok(()) => Ok(()),
        Err(e) => {
            helper.cancel();
            Err(e)
        }
    }
}

/// Step 5: the `vitals relaunch` helper, waiting for this process to exit.
pub struct Helper(Child);

impl Helper {
    pub fn spawn(parent: u32, app_url: &Path) -> Result<Helper, UpdateError> {
        let exe = std::env::current_exe()?;
        let child = Command::new(exe)
            .args(["relaunch", "--parent", &parent.to_string(), "--app"])
            .arg(app_url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| UpdateError::Io(format!("spawning the relaunch helper: {e}")))?;
        Ok(Helper(child))
    }

    /// SIGTERM the helper (it is blocked in `kevent`, so this is the only
    /// way to stop it) and reap it.
    pub fn cancel(mut self) {
        // SAFETY: `id()` is the pid of a child this process spawned and has
        // not yet reaped, so it cannot have been reused.
        unsafe { libc::kill(self.0.id() as libc::pid_t, libc::SIGTERM) };
        let _ = self.0.wait();
    }
}

/// Step 1: a fresh directory on the same volume as `app_url`, created by
/// Foundation for exactly this purpose (`NSItemReplacementDirectory`).
fn staging_dir(app_url: &Path) -> Result<PathBuf, UpdateError> {
    let near = file_url(app_url);
    let url = NSFileManager::defaultManager()
        .URLForDirectory_inDomain_appropriateForURL_create_error(
            NSSearchPathDirectory::ItemReplacementDirectory,
            NSSearchPathDomainMask::UserDomainMask,
            Some(&near),
            true,
        )
        .map_err(|e| {
            UpdateError::Io(format!(
                "creating a staging directory next to {}: {}",
                app_url.display(),
                e.localizedDescription()
            ))
        })?;
    let path = url
        .path()
        .ok_or_else(|| UpdateError::Io("staging directory has no path".into()))?;
    Ok(PathBuf::from(path.to_string()))
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn sha256_of(path: &Path) -> Result<[u8; 32], UpdateError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Unpack `archive` into `into`, requiring its top level to be exactly
/// `Vitals.app`. Two passes over the gzip stream: a listing, then the
/// unpack, so nothing is written before the shape is known.
fn unpack(archive: &Path, into: &Path) -> Result<(), UpdateError> {
    let mut top = BTreeSet::new();
    let mut listing = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
    for entry in listing.entries()? {
        let entry = entry?;
        let path = entry.path()?;
        let first = path
            .components()
            .find_map(|c| match c {
                std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .ok_or_else(|| UpdateError::BadRelease(format!("archive entry with no name: {}", path.display())))?;
        top.insert(first);
    }
    if top.len() != 1 || !top.contains(BUNDLE_NAME) {
        return Err(UpdateError::BadRelease(format!(
            "archive top level is {top:?}, expected exactly [{BUNDLE_NAME:?}]"
        )));
    }

    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
    // Nothing in the fresh staging directory may be replaced, and the crate
    // rejects entries that would escape `into`. Extended attributes are not
    // unpacked (the default), so no quarantine flag arrives with the bytes;
    // modes are applied masked to 0o777, so executable bits do.
    archive.set_overwrite(false);
    archive.unpack(into)?;
    Ok(())
}

/// `CFBundleShortVersionString` of the bundle at `app`.
fn bundle_version(app: &Path) -> Result<Version, UpdateError> {
    let plist = app.join("Contents/Info.plist");
    let url = file_url(&plist);
    // SAFETY: `url` is a file URL; Foundation returns `None` for anything
    // that is not a property list rather than trusting the bytes.
    let dict: Retained<NSDictionary<NSString, AnyObject>> =
        unsafe { NSDictionary::dictionaryWithContentsOfURL(&url) }.ok_or_else(|| {
            UpdateError::BadRelease(format!("{} is missing or not a property list", plist.display()))
        })?;
    let value = dict
        .objectForKey(&NSString::from_str("CFBundleShortVersionString"))
        .ok_or_else(|| UpdateError::BadRelease("Info.plist has no CFBundleShortVersionString".into()))?;
    let text = value
        .downcast_ref::<NSString>()
        .ok_or_else(|| UpdateError::BadRelease("CFBundleShortVersionString is not a string".into()))?
        .to_string();
    Version::parse(&text).map_err(|e| UpdateError::BadRelease(format!("CFBundleShortVersionString: {e}")))
}
```

Add `pub mod install;` to `update/mod.rs`.

- [ ] **Step 4: Run the tests**

```bash
cargo test -p vitals --test update -- --nocapture 2>&1 | tail -12
```

Expected: `test result: ok. 5 passed`, and one `vitals: the running copy has no Team ID (ad-hoc signature); skipping the signature check on Vitals 0.1.1` line per test that reaches step 4 (the happy path and the version-mismatch case do not reach it — the mismatch fails first — so exactly one line, from the happy path).

If `a_good_release_replaces_the_installed_bundle` fails at `swap_into` with a "cross-device" or "not permitted" error, the staging directory did not land on the tempdir's volume — check that `staging_dir` passed `Some(&near)` and not `None`.

- [ ] **Step 5: Lint, format, commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets --examples -- -D warnings 2>&1 | tail -2
git add crates/app/src/update/install.rs crates/app/src/update/mod.rs crates/app/tests/update.rs
git commit -m "feat(update): stage, download, verify, unpack and swap an update

prepare stages in Foundation's item-replacement directory next to the
bundle, downloads SHA256SUMS and the tarball, checks the SHA-256, unpacks
with overwrite off and xattrs off, requires exactly one top-level
Vitals.app whose Info.plist carries the release version, and verifies
the Developer ID signature against the running copy's Team ID when it has
one. Staged::swap_into replaces the installed bundle with
replaceItemAtURL; Drop removes the staging directory otherwise. Helper
spawns \`vitals relaunch\`. Five integration tests drive it against a
tiny_http fixture serving a tarball the test builds: replaced, and four
kinds of refusal that leave the installed bundle byte-for-byte unchanged.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: `update::relaunch` and the hidden `vitals relaunch` verb

Section §4 step 5 (what the helper does) and "CLI additions" (`Command::Relaunch`).

**Files:**
- Create: `crates/app/src/update/relaunch.rs`
- Modify: `crates/app/src/update/mod.rs` (add `pub mod relaunch;`)
- Modify: `crates/app/src/window/mod.rs:20` (`mod parent;` → `pub(crate) mod parent;`)
- Modify: `crates/app/src/window/parent.rs:25` (`fn wait_for_exit` → `pub(crate) fn wait_for_exit`)
- Modify: `crates/app/src/cli.rs` (new `Command::Relaunch`)
- Modify: `crates/app/src/main.rs` (dispatch)
- Create: `crates/app/tests/relaunch.rs`

**Interfaces:**
- Consumes: `window::parent::wait_for_exit(pid: u32) -> bool` (made `pub(crate)`).
- Produces: `pub fn run(parent: u32, app: &Path) -> Result<(), String>`; `Command::Relaunch { parent: u32, app: PathBuf }`.

Binding facts (verified in `objc2-app-kit 0.3.2`): `NSWorkspaceOpenConfiguration::configuration()`, `setCreatesNewApplicationInstance(bool)`, `setActivates(bool)` — safe; `NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(&NSURL, &cfg, Option<&DynBlock<dyn Fn(*mut NSRunningApplication, *mut NSError)>>)` — safe (the exact call `tray/child.rs` makes); `NSRunLoop::mainRunLoop().runUntilDate(&NSDate)`; `NSDate::dateWithTimeIntervalSinceNow(f64)`.

- [ ] **Step 1: Write the failing subprocess tests**

Create `crates/app/tests/relaunch.rs`:

```rust
//! The relaunch helper as a subprocess. Neither test launches anything:
//! one names a bundle that does not exist, the other a directory that is
//! not an app, so LaunchServices answers with an error both times. The
//! second is the only automated proof that the completion handler fires
//! in a process that never runs `NSApplication`.

use std::process::Command;
use std::time::{Duration, Instant};

fn relaunch(parent: u32, app: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(["relaunch", "--parent", &parent.to_string(), "--app", app])
        .output()
        .expect("run vitals relaunch")
}

#[test]
fn the_helper_waits_for_its_parent_before_doing_anything() {
    let mut parent = Command::new("sleep").arg("0.5").spawn().unwrap();
    let started = Instant::now();
    let out = relaunch(parent.id(), "/nonexistent/Vitals.app");
    let took = started.elapsed();
    let _ = parent.wait();

    assert!(took >= Duration::from_millis(400), "returned after {took:?}, before the parent exited");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("does not exist"), "{stderr}");
}

#[test]
fn a_parent_that_is_already_gone_is_not_waited_for() {
    let mut parent = Command::new("true").spawn().unwrap();
    let pid = parent.id();
    let _ = parent.wait(); // reaped: the pid is gone before the helper looks
    let started = Instant::now();
    let out = relaunch(pid, "/nonexistent/Vitals.app");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!out.status.success());
}

#[test]
fn a_path_that_is_not_an_app_is_refused_by_launch_services() {
    let dir = tempfile::tempdir().unwrap();
    let mut parent = Command::new("sleep").arg("0.2").spawn().unwrap();
    let out = relaunch(parent.id(), &dir.path().to_string_lossy());
    let _ = parent.wait();

    assert!(!out.status.success(), "opening a plain directory must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.starts_with("vitals: "), "{stderr}");
    assert!(stderr.len() > "vitals: ".len(), "LaunchServices' reason should be in the message: {stderr}");
}
```

- [ ] **Step 2: Run them to see them fail**

```bash
cargo test -p vitals --test relaunch 2>&1 | grep -E "^error|panicked|unrecognized" | head -3
```

Expected: `relaunch` is an unrecognized subcommand — the helper exits 2 with clap's usage on stderr, so the assertions on stderr content fail.

- [ ] **Step 3: Expose `wait_for_exit`**

In `crates/app/src/window/mod.rs`, change `mod parent;` to `pub(crate) mod parent;`. In `crates/app/src/window/parent.rs`, change `fn wait_for_exit(pid: u32) -> bool` to `pub(crate) fn wait_for_exit(pid: u32) -> bool` and extend its doc comment's last paragraph: "Split out from `watch` so a test can call it directly without going through `std::process::exit`, and shared with `update::relaunch`, which waits on the tray the same way."

- [ ] **Step 4: Write `crates/app/src/update/relaunch.rs`**

```rust
//! The body of the hidden `vitals relaunch` verb: wait for the tray that
//! spawned this process to exit, then open the bundle it installed.
//!
//! Spawned by `update::install::Helper` just before the swap. The wait is
//! the same kqueue registration the dashboard window uses to follow the
//! tray (`window::parent`): zero CPU, one wakeup. The launch goes through
//! LaunchServices rather than `exec`, for the same reasons `tray/child.rs`
//! gives — and because the new bundle must start as its own process, not
//! as a child of this one.

use std::path::Path;
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSDate, NSError, NSRunLoop, NSString, NSURL};

/// How long to wait for LaunchServices to answer once the old process is
/// gone. It normally answers within a second.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Wait for `parent` to exit, then open `app`.
pub fn run(parent: u32, app: &Path) -> Result<(), String> {
    // A pid that is already gone returns at once — see `wait_for_exit`.
    crate::window::parent::wait_for_exit(parent);
    launch(app)
}

fn launch(app: &Path) -> Result<(), String> {
    if !app.exists() {
        return Err(format!("{} does not exist", app.display()));
    }
    let url = NSURL::fileURLWithPath(&NSString::from_str(&app.to_string_lossy()));
    let config = NSWorkspaceOpenConfiguration::configuration();
    // A new instance, not a re-front of anything already running under the
    // same bundle identifier; and no activation, because a menu bar app
    // has nothing to bring forward.
    config.setCreatesNewApplicationInstance(true);
    config.setActivates(false);

    let (tx, rx) = mpsc::channel::<Result<i32, String>>();
    let handler = block2::RcBlock::new(
        move |app: *mut NSRunningApplication, error: *mut NSError| {
            let result = if let Some(app) = NonNull::new(app) {
                // SAFETY: a non-null pointer handed to this completion
                // handler by AppKit is a valid `NSRunningApplication` for
                // the duration of this call.
                Ok(unsafe { app.as_ref() }.processIdentifier())
            } else if let Some(error) = NonNull::new(error) {
                // SAFETY: same guarantee, for the error arm.
                Err(unsafe { error.as_ref() }.localizedDescription().to_string())
            } else {
                Err("LaunchServices returned neither an application nor an error".to_string())
            };
            let _ = tx.send(result);
        },
    );
    NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(
        &url,
        &config,
        Some(&handler),
    );

    // The completion handler arrives on a LaunchServices queue, but this
    // process has no `NSApplication` and nothing else drives its main run
    // loop, so drive it here in short slices until the answer lands.
    let deadline = Instant::now() + LAUNCH_TIMEOUT;
    loop {
        if let Ok(result) = rx.try_recv() {
            return result.map(|pid| eprintln!("vitals: relaunched {} as pid {pid}", app.display()));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "LaunchServices did not answer within {}s for {}",
                LAUNCH_TIMEOUT.as_secs(),
                app.display()
            ));
        }
        NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.1));
    }
}
```

Add `pub mod relaunch;` to `update/mod.rs`.

- [ ] **Step 5: Add the verb**

In `crates/app/src/cli.rs`, add `use std::path::PathBuf;` at the top and this variant after `Window`:

```rust
    /// Wait for the tray that spawned this process to exit, then open the
    /// bundle it just installed.
    ///
    /// Spawned by the in-app updater a moment before it swaps the bundle;
    /// not meant to be run by hand, and hidden from `--help` for that
    /// reason.
    #[command(hide = true)]
    Relaunch {
        /// Pid of the tray to wait for.
        #[arg(long)]
        parent: u32,
        /// The bundle to open once it is gone.
        #[arg(long)]
        app: PathBuf,
    },
```

In `crates/app/src/main.rs`, import `update` alongside the others (`use vitals::{serve, tray, update, window};`) and add the arm before `None`:

```rust
        Some(Command::Relaunch { parent, app }) => update::relaunch::run(parent, &app),
```

- [ ] **Step 6: Run the tests**

```bash
cargo test -p vitals --test relaunch -- --nocapture 2>&1 | tail -6
```

Expected: `test result: ok. 3 passed`. The second test proves the "parent already gone" path returns promptly; the third proves the completion handler fires without `NSApplication`. **If the third test fails because the helper exits 0 — LaunchServices opened the folder in Finder rather than refusing it — stop and report `DONE_WITH_CONCERNS` with the exact stderr; do not weaken the test.**

Also confirm the verb is hidden:

```bash
cargo run -q -p vitals -- --help | grep -c relaunch
```

Expected: `0`.

- [ ] **Step 7: Lint, format, commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets --examples -- -D warnings 2>&1 | tail -2
git add crates/app/src/update/relaunch.rs crates/app/src/update/mod.rs crates/app/src/window/mod.rs crates/app/src/window/parent.rs crates/app/src/cli.rs crates/app/src/main.rs crates/app/tests/relaunch.rs
git commit -m "feat(update): hidden \`vitals relaunch\` helper

Waits for the tray's pid with the same kqueue registration the dashboard
window uses, then opens the installed bundle through LaunchServices as a
new instance, pumping the main run loop until the completion handler
answers. Tested as a subprocess: it waits for its parent, it does not
wait for a parent that is already gone, and a path that is not an app is
refused with LaunchServices' reason on stderr.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: The checker, its schedule, and `Check for Updates…`

Section §2 "Schedule", "The request" (the hand-off), "State and decision", "Failure"; §3 item 6 (`Check for Updates…`); §4 "CLI additions" (`--update-source`).

**Files:**
- Create: `crates/app/src/update/checker.rs`
- Modify: `crates/app/src/update/mod.rs` (add `pub mod checker;`)
- Create: `crates/app/src/tray/update_ui.rs`
- Modify: `crates/app/src/tray/mod.rs`
- Modify: `crates/app/src/tray/controller.rs`
- Modify: `crates/app/src/cli.rs`
- Modify: `crates/app/src/main.rs`
- Create: `crates/app/tests/update_source.rs`

**Interfaces:**
- Consumes: `http::fetch`, `release::{parse_latest, Release, Source, Version}`.
- Produces (used by Tasks 9–11):
  - `checker`: `pub enum CheckOutcome { UpToDate(Version), Available(Release), Failed(String) }`; `pub struct UpdateState { pub available: Option<Release>, pub checking: bool, pub installing: bool, pub last_check: Option<(Instant, Result<(), String>)> }` with `fn apply(&mut self, CheckOutcome, Instant)`; `pub type Slot<T> = Arc<Mutex<Option<T>>>`; `pub fn check_now(&Source, Version) -> CheckOutcome`; `pub fn spawn_check(Source, Version, manual: bool, Slot<(CheckOutcome, bool)>, notify: impl FnOnce() + Send + 'static)`; constants `FIRST_CHECK_S = 30.0`, `CHECK_PERIOD_S = 86400.0`, `CHECK_TOLERANCE_S = 3600.0`.
  - `tray::update_ui`: `pub(super) struct UpdateIvars { source: Option<Source>, state: RefCell<UpdateState>, check_slot, timers, items: Option<UpdateItems> }` with `UpdateIvars::new(mtm, Source)`; `pub(super) struct UpdateItems { row, row_separator, check }`; `pub(super) const CHECK_DONE: &str`; `pub(super) fn is_bundled() -> bool`, `app_url() -> PathBuf`, `post(&str)`, `alert(mtm, &str, &str)`, `open_page(&str)`; `pub fn truncate_notes(&str) -> String`; on `Controller`: `install_update_items(&NSMenu)`, `schedule_update_checks()`, `start_check(manual)`, `check_finished()`, `offer_install(&Release)`, `install_update()`, `refresh_update_items()`.
  - `controller::Ivars` fields `status_item` and `update` become `pub(super)`; `Controller::new(mtm, source: Source)`.
  - `tray::run(source: Source) -> !`; `cli::update_source(Option<String>) -> Result<Source, String>`; `Cli.update_source: Option<String>` (hidden flag).

- [ ] **Step 1: Write the failing checker tests**

Create `crates/app/src/update/checker.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::release::archive_name;
    use std::sync::{Arc, Mutex};

    fn release(version: &str) -> Release {
        let version = Version::parse(version).unwrap();
        Release {
            version,
            notes: String::new(),
            page_url: String::new(),
            archive_url: String::new(),
            sums_url: String::new(),
            archive_name: archive_name(version),
        }
    }

    #[test]
    fn a_newer_release_becomes_available() {
        let mut state = UpdateState { checking: true, ..Default::default() };
        state.apply(CheckOutcome::Available(release("0.2.0")), Instant::now());
        assert_eq!(state.available.as_ref().map(|r| r.version.to_string()), Some("0.2.0".into()));
        assert!(!state.checking);
        assert!(matches!(state.last_check, Some((_, Ok(())))));
    }

    #[test]
    fn a_newer_latest_replaces_an_older_available() {
        let mut state = UpdateState::default();
        state.apply(CheckOutcome::Available(release("0.2.0")), Instant::now());
        state.apply(CheckOutcome::Available(release("0.3.0")), Instant::now());
        assert_eq!(state.available.as_ref().map(|r| r.version.to_string()), Some("0.3.0".into()));
    }

    #[test]
    fn up_to_date_clears_available() {
        let mut state = UpdateState::default();
        state.apply(CheckOutcome::Available(release("0.2.0")), Instant::now());
        state.apply(CheckOutcome::UpToDate(Version::parse("0.1.0").unwrap()), Instant::now());
        assert!(state.available.is_none());
        assert!(!state.checking);
    }

    #[test]
    fn a_failure_keeps_available_and_records_the_reason() {
        let mut state = UpdateState { checking: true, ..Default::default() };
        state.apply(CheckOutcome::Available(release("0.2.0")), Instant::now());
        state.apply(CheckOutcome::Failed("offline".into()), Instant::now());
        assert!(state.available.is_some(), "a failed check must not forget a known update");
        assert!(!state.checking);
        assert!(matches!(&state.last_check, Some((_, Err(r))) if r == "offline"));
    }

    /// A loopback server answering `/releases/latest` with `body` at `status`.
    struct Fixture {
        base: String,
        server: Arc<tiny_http::Server>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    fn serve_latest(status: u16, body: Vec<u8>) -> Fixture {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let s = Arc::clone(&server);
        let thread = std::thread::spawn(move || {
            for request in s.incoming_requests() {
                let _ = request.respond(tiny_http::Response::from_data(body.clone()).with_status_code(status));
            }
        });
        Fixture { base: format!("http://127.0.0.1:{port}/"), server, thread: Some(thread) }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.unblock();
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn latest_json(base: &str, version: &str) -> Vec<u8> {
        let name = archive_name(Version::parse(version).unwrap());
        serde_json::to_vec(&serde_json::json!({
            "tag_name": format!("v{version}"), "draft": false, "prerelease": false,
            "html_url": format!("{base}releases/tag/v{version}"), "body": "notes",
            "assets": [
                {"name": name, "browser_download_url": format!("{base}{name}")},
                {"name": "SHA256SUMS", "browser_download_url": format!("{base}SHA256SUMS")}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn check_now_reports_newer_same_and_broken() {
        let current = Version::parse("0.1.0").unwrap();

        let fx = serve_latest(200, Vec::new());
        let newer = serve_latest(200, latest_json(&fx.base, "0.2.0"));
        drop(fx);
        let outcome = check_now(&Source::loopback(&newer.base).unwrap(), current);
        assert!(matches!(&outcome, CheckOutcome::Available(r) if r.version.to_string() == "0.2.0"), "{outcome:?}");

        let same = serve_latest(200, latest_json(&newer.base, "0.1.0"));
        let outcome = check_now(&Source::loopback(&same.base).unwrap(), current);
        assert!(matches!(&outcome, CheckOutcome::UpToDate(v) if v.to_string() == "0.1.0"), "{outcome:?}");

        let broken = serve_latest(503, b"nope".to_vec());
        let outcome = check_now(&Source::loopback(&broken.base).unwrap(), current);
        assert!(matches!(&outcome, CheckOutcome::Failed(r) if r.contains("503")), "{outcome:?}");
    }

    #[test]
    fn spawn_check_fills_the_slot_then_notifies() {
        let fx = serve_latest(404, Vec::new());
        let slot: Slot<(CheckOutcome, bool)> = Arc::new(Mutex::new(None));
        let (tx, rx) = std::sync::mpsc::channel();
        spawn_check(Source::loopback(&fx.base).unwrap(), Version::parse("0.1.0").unwrap(), true, Arc::clone(&slot), move || {
            let _ = tx.send(());
        });
        rx.recv_timeout(std::time::Duration::from_secs(20)).expect("notified");
        let (outcome, manual) = slot.lock().unwrap().take().expect("slot filled before notify");
        assert!(manual);
        assert!(matches!(outcome, CheckOutcome::Failed(_)));
    }
}
```

The `newer` fixture embeds `fx.base` in its JSON and then serves it from a different port — the JSON's asset URLs must start with the *served* base, so that first block builds the JSON with `newer.base`... it cannot, because `newer` does not exist until the JSON exists. Replace that block with the two-step form the integration test uses:

```rust
        let newer = {
            let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
            let port = server.server_addr().to_ip().unwrap().port();
            let base = format!("http://127.0.0.1:{port}/");
            let body = latest_json(&base, "0.2.0");
            let s = Arc::clone(&server);
            let thread = std::thread::spawn(move || {
                for request in s.incoming_requests() {
                    let _ = request.respond(tiny_http::Response::from_data(body.clone()));
                }
            });
            Fixture { base, server, thread: Some(thread) }
        };
```

and likewise for `same` (with `"0.1.0"`). Delete the `let fx = serve_latest(200, Vec::new()); … drop(fx);` lines. `serve_latest` stays for the two status-only cases.

- [ ] **Step 2: Run them to see them fail**

```bash
cargo test -p vitals --lib update::checker 2>&1 | grep -E "^error" | head -3
```

Expected: compile errors, `UpdateState` etc. not found.

- [ ] **Step 3: Write the checker above the tests**

```rust
//! One update check, and the state the tray keeps about updates.
//!
//! `check_now` blocks and belongs on a worker thread; `spawn_check` puts
//! it there and, when it is done, leaves the outcome in a slot the main
//! thread reads and then calls `notify`, which the tray uses to post the
//! notification its controller observes. `UpdateState::apply` is the pure
//! decision about what an outcome means, and is unit-tested.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::http;
use super::release::{parse_latest, Release, Source, Version};

/// Seconds after the tray starts before the first check.
pub const FIRST_CHECK_S: f64 = 30.0;

/// Seconds between checks after the first: one a day.
pub const CHECK_PERIOD_S: f64 = 24.0 * 60.0 * 60.0;

/// Slack macOS may add to the daily check to coalesce it with other
/// wake-ups. An hour late is fine for a once-a-day question.
pub const CHECK_TOLERANCE_S: f64 = 3600.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The latest release is this version, which is not newer than ours.
    UpToDate(Version),
    Available(Release),
    Failed(String),
}

/// Ask `source` for the latest release and compare it with `current`.
pub fn check_now(source: &Source, current: Version) -> CheckOutcome {
    let json = match http::fetch(&source.api_latest) {
        Ok(body) => body,
        Err(e) => return CheckOutcome::Failed(e.to_string()),
    };
    match parse_latest(&json, source) {
        Ok(release) if release.version > current => CheckOutcome::Available(release),
        Ok(release) => CheckOutcome::UpToDate(release.version),
        Err(e) => CheckOutcome::Failed(e.to_string()),
    }
}

/// What the tray knows about updates. Lives in the controller's ivars and
/// is touched only on the main thread.
#[derive(Debug, Default)]
pub struct UpdateState {
    /// A release newer than the running one, if the last successful check
    /// found one.
    pub available: Option<Release>,
    pub checking: bool,
    pub installing: bool,
    pub last_check: Option<(Instant, Result<(), String>)>,
}

impl UpdateState {
    /// Fold a finished check in. A newer "latest" replaces `available`; an
    /// equal or older one clears it; a failure leaves it alone. Every
    /// outcome ends the check.
    pub fn apply(&mut self, outcome: CheckOutcome, now: Instant) {
        self.checking = false;
        match outcome {
            CheckOutcome::Available(release) => {
                self.available = Some(release);
                self.last_check = Some((now, Ok(())));
            }
            CheckOutcome::UpToDate(_) => {
                self.available = None;
                self.last_check = Some((now, Ok(())));
            }
            CheckOutcome::Failed(reason) => {
                self.last_check = Some((now, Err(reason)));
            }
        }
    }
}

/// Where a worker leaves its result for the main thread.
pub type Slot<T> = Arc<Mutex<Option<T>>>;

/// Run `check_now` on a named thread; store `(outcome, manual)` in `slot`,
/// then call `notify`. `manual` rides along so the main thread knows
/// whether to report the outcome in an alert.
pub fn spawn_check(
    source: Source,
    current: Version,
    manual: bool,
    slot: Slot<(CheckOutcome, bool)>,
    notify: impl FnOnce() + Send + 'static,
) {
    std::thread::Builder::new()
        .name("vitals-update-check".into())
        .spawn(move || {
            let outcome = check_now(&source, current);
            *slot.lock().unwrap() = Some((outcome, manual));
            notify();
        })
        .expect("spawning the update check thread");
}
```

Add `pub mod checker;` to `update/mod.rs`.

```bash
cargo test -p vitals --lib update::checker 2>&1 | tail -3
```

Expected: `test result: ok. 6 passed`.

- [ ] **Step 4: The CLI flag and its test**

In `crates/app/src/cli.rs`, add to the `Cli` struct:

```rust
    /// Where the menu bar app looks for updates: a loopback base such as
    /// `http://127.0.0.1:8000/`, for the manual end-to-end test in
    /// CONTRIBUTING.md. Anything else is refused. Hidden from `--help`.
    #[arg(long, hide = true)]
    pub update_source: Option<String>,
```

and, after `collect_snapshot`, the function and its import (`use crate::update::release::Source;` at the top):

```rust
/// The release source for the tray: GitHub, unless the hidden
/// `--update-source` flag names a loopback base.
pub fn update_source(flag: Option<String>) -> Result<Source, String> {
    match flag {
        None => Ok(Source::github()),
        Some(url) => Source::loopback(&url).map_err(|e| e.to_string()),
    }
}
```

and in `cli.rs`'s `mod tests`:

```rust
    #[test]
    fn update_source_defaults_to_github_and_accepts_only_loopback() {
        use clap::CommandFactory;
        assert_eq!(update_source(None), Ok(Source::github()));
        assert_eq!(
            update_source(Some("http://localhost:8000/".into())).map(|s| s.download_base),
            Ok("http://localhost:8000/".to_string())
        );
        let err = update_source(Some("https://example.com/".into())).unwrap_err();
        assert!(err.contains("https://example.com/"), "{err}");
        // Parses on the bare invocation, and stays out of --help.
        let cli = Cli::try_parse_from(["vitals", "--update-source", "http://127.0.0.1:1/"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.update_source.as_deref(), Some("http://127.0.0.1:1/"));
        assert!(!Cli::command().render_help().to_string().contains("update-source"));
    }
```

Create `crates/app/tests/update_source.rs`, which proves a bad source is refused before any AppKit object exists (the process exits 1 at once instead of becoming a menu bar item):

```rust
//! `vitals --update-source <bad>` must fail before the tray starts.

use std::process::Command;

#[test]
fn a_non_loopback_update_source_is_refused_before_the_tray_starts() {
    let out = Command::new(env!("CARGO_BIN_EXE_vitals"))
        .args(["--update-source", "https://example.com/"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("update source must be"), "{stderr}");
}
```

- [ ] **Step 5: Write `crates/app/src/tray/update_ui.rs`**

```rust
//! The controller's update behaviour: the check schedule, the menu rows,
//! the alerts. The Objective-C selectors themselves live in
//! `controller.rs` — they must be inside `define_class!` — and each is one
//! line calling into here; this file is where the logic is.
//!
//! Everything here runs on the main thread. The worker threads (a check,
//! and later an install) never touch `self`: they leave their result in a
//! `Slot` and post a notification, and the controller hops back onto the
//! main thread before reading it — the `powerChanged:` pattern.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use objc2::rc::Retained;
use objc2::sel;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSMenu, NSMenuItem, NSWorkspace};
use objc2_foundation::{
    MainThreadMarker, NSBundle, NSNotificationCenter, NSRunLoop, NSRunLoopCommonModes, NSString,
    NSTimer, NSURL,
};

use crate::update::checker::{
    self, CheckOutcome, Slot, UpdateState, CHECK_PERIOD_S, CHECK_TOLERANCE_S, FIRST_CHECK_S,
};
use crate::update::release::{Release, Source, Version};

use super::controller::Controller;

/// Posted from the check thread once its outcome is in the slot.
pub(super) const CHECK_DONE: &str = "com.billsun.vitals.updateCheckDone";

/// Whether this process runs from a `.app` bundle. Only then does the
/// feature exist: a `cargo run` binary has no bundle to replace.
pub(super) fn is_bundled() -> bool {
    NSBundle::mainBundle()
        .bundleURL()
        .path()
        .is_some_and(|p| p.to_string().ends_with(".app"))
}

/// The bundle's path, for the installer. Meaningful only when `is_bundled()`.
pub(super) fn app_url() -> PathBuf {
    PathBuf::from(NSBundle::mainBundle().bundlePath().to_string())
}

/// Post `name` on the default centre. Called from worker threads: the
/// observer runs on the posting thread and only hops, so this is safe to
/// call from anywhere.
pub(super) fn post(name: &str) {
    // SAFETY: the name is one of this module's own constants; no object.
    unsafe {
        NSNotificationCenter::defaultCenter()
            .postNotificationName_object(&NSString::from_str(name), None)
    }
}

/// An informational alert with one OK button.
pub(super) fn alert(mtm: MainThreadMarker, message: &str, informative: &str) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str(message));
    alert.setInformativeText(&NSString::from_str(informative));
    alert.runModal();
}

/// Open a web page in the default browser.
pub(super) fn open_page(url: &str) {
    match NSURL::URLWithString(&NSString::from_str(url)) {
        Some(ns_url) if NSWorkspace::sharedWorkspace().openURL(&ns_url) => {}
        _ => eprintln!("vitals: could not open {url}"),
    }
}

/// Release notes as the alert shows them: at most 1,500 characters, then `…`.
pub fn truncate_notes(notes: &str) -> String {
    const LIMIT: usize = 1_500;
    let trimmed = notes.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_string();
    }
    let mut cut: String = trimmed.chars().take(LIMIT).collect();
    cut.push('…');
    cut
}

/// The menu items this feature owns. Created before `init` so they can
/// live in the ivars; targeted at `self` after (`install_update_items`).
pub(super) struct UpdateItems {
    /// `● Update to Vitals x.y.z…`, present only while an update is available.
    pub(super) row: Retained<NSMenuItem>,
    /// The separator under `row`, present with it.
    pub(super) row_separator: Retained<NSMenuItem>,
    /// `Check for Updates…`.
    pub(super) check: Retained<NSMenuItem>,
}

pub(super) struct UpdateIvars {
    /// `None` outside a bundle: no timers, no items, nothing.
    pub(super) source: Option<Source>,
    pub(super) state: RefCell<UpdateState>,
    pub(super) check_slot: Slot<(CheckOutcome, bool)>,
    /// The 30 s one-shot and the daily repeat, kept so they are not
    /// collected. Never invalidated: they live as long as the tray.
    pub(super) timers: RefCell<Vec<Retained<NSTimer>>>,
    pub(super) items: Option<UpdateItems>,
}

impl UpdateIvars {
    pub(super) fn new(mtm: MainThreadMarker, source: Source) -> UpdateIvars {
        let bundled = is_bundled();
        let items = bundled.then(|| {
            let check = NSMenuItem::new(mtm);
            check.setTitle(&NSString::from_str("Check for Updates…"));
            UpdateItems {
                row: NSMenuItem::new(mtm),
                row_separator: NSMenuItem::separatorItem(mtm),
                check,
            }
        });
        UpdateIvars {
            source: bundled.then_some(source),
            state: RefCell::new(UpdateState::default()),
            check_slot: Arc::new(Mutex::new(None)),
            timers: RefCell::new(Vec::new()),
            items,
        }
    }
}

impl Controller {
    /// Append `Check for Updates…` to `menu` and point the items at `self`.
    /// Runs after `init`. No-op outside a bundle.
    pub(super) fn install_update_items(&self, menu: &NSMenu) {
        let Some(items) = self.ivars().update.items.as_ref() else {
            return;
        };
        items.check.setEnabled(true);
        // SAFETY: `self` responds to `checkForUpdates:` and `installUpdate:`,
        // defined in `controller.rs`.
        unsafe {
            items.check.setTarget(Some(self));
            items.check.setAction(Some(sel!(checkForUpdates:)));
            items.row.setTarget(Some(self));
            items.row.setAction(Some(sel!(installUpdate:)));
        }
        menu.addItem(&items.check);
    }

    /// The 30 s one-shot and the daily repeat, in common modes like the
    /// poll timer, so an open menu does not stall them. No-op outside a
    /// bundle.
    pub(super) fn schedule_update_checks(&self) {
        if self.ivars().update.source.is_none() {
            return;
        }
        let mut timers = Vec::new();
        for (interval, repeats, tolerance) in [
            (FIRST_CHECK_S, false, 5.0),
            (CHECK_PERIOD_S, true, CHECK_TOLERANCE_S),
        ] {
            // SAFETY: `self` responds to `updateTimerFired:`; there is no
            // user info; the mode is Foundation's own constant.
            let timer = unsafe {
                NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                    interval,
                    self,
                    sel!(updateTimerFired:),
                    None,
                    repeats,
                )
            };
            timer.setTolerance(tolerance);
            unsafe { NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
            timers.push(timer);
        }
        *self.ivars().update.timers.borrow_mut() = timers;
    }

    /// Start a check unless one is already running or an install is.
    /// `manual` decides whether the outcome is reported in an alert.
    pub(super) fn start_check(&self, manual: bool) {
        let Some(source) = self.ivars().update.source.clone() else {
            return;
        };
        {
            let mut state = self.ivars().update.state.borrow_mut();
            if state.checking || state.installing {
                return;
            }
            state.checking = true;
        }
        let slot = Arc::clone(&self.ivars().update.check_slot);
        checker::spawn_check(source, Version::current(), manual, slot, || post(CHECK_DONE));
    }

    /// On the main thread, after `CHECK_DONE`: fold the outcome in and,
    /// for a manual check, say what happened. An automatic failure is one
    /// line on stderr and nothing else.
    pub(super) fn check_finished(&self) {
        let Some((outcome, manual)) = self.ivars().update.check_slot.lock().unwrap().take() else {
            return;
        };
        if let (CheckOutcome::Failed(reason), false) = (&outcome, manual) {
            eprintln!("vitals: update check failed: {reason}");
        }
        self.ivars().update.state.borrow_mut().apply(outcome.clone(), Instant::now());
        if !manual {
            return;
        }
        let mtm = MainThreadMarker::from(self);
        match outcome {
            CheckOutcome::UpToDate(latest) => {
                alert(mtm, "You're up to date", &format!("Vitals {latest} is the latest."))
            }
            CheckOutcome::Failed(reason) => alert(mtm, "Couldn't check for updates", &reason),
            CheckOutcome::Available(release) => self.offer_install(&release),
        }
    }

    /// The row's action: offer the update that is available.
    pub(super) fn install_update(&self) {
        let available = self.ivars().update.state.borrow().available.clone();
        if let Some(release) = available {
            self.offer_install(&release);
        }
    }

    /// The release alert. *View Release* opens the release page.
    pub(super) fn offer_install(&self, release: &Release) {
        let mtm = MainThreadMarker::from(self);
        let alert = NSAlert::new(mtm);
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str(&format!("Vitals {} is available", release.version)));
        alert.setInformativeText(&NSString::from_str(&truncate_notes(&release.notes)));
        alert.addButtonWithTitle(&NSString::from_str("View Release"));
        alert.addButtonWithTitle(&NSString::from_str("Later"));
        if alert.runModal() == NSAlertFirstButtonReturn {
            open_page(&release.page_url);
        }
    }

    /// Bring the rows in line with the state. Called from `menuWillOpen:`,
    /// so nothing is touched while the menu is closed.
    pub(super) fn refresh_update_items(&self) {
        let Some(items) = self.ivars().update.items.as_ref() else {
            return;
        };
        let Some(menu) = self.ivars().status_item.menu(MainThreadMarker::from(self)) else {
            return;
        };
        let state = self.ivars().update.state.borrow();
        let present = menu.indexOfItem(&items.row) >= 0;
        match (&state.available, present) {
            (Some(release), _) => {
                if !present {
                    menu.insertItem_atIndex(&items.row_separator, 0);
                    menu.insertItem_atIndex(&items.row, 0);
                }
                items.row.setTitle(&NSString::from_str(&format!("● Update to Vitals {}…", release.version)));
                items.row.setEnabled(true);
            }
            (None, true) => {
                menu.removeItem(&items.row);
                menu.removeItem(&items.row_separator);
            }
            (None, false) => {}
        }
        let (title, enabled) = if state.checking {
            ("Checking…", false)
        } else {
            ("Check for Updates…", true)
        };
        items.check.setTitle(&NSString::from_str(title));
        items.check.setEnabled(enabled);
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_notes;

    #[test]
    fn notes_are_trimmed_and_cut_at_fifteen_hundred_characters() {
        assert_eq!(truncate_notes("  ### Added\n- x\n"), "### Added\n- x");
        let exact: String = "é".repeat(1_500);
        assert_eq!(truncate_notes(&exact), exact);
        let long: String = "é".repeat(1_501);
        let cut = truncate_notes(&long);
        assert_eq!(cut.chars().count(), 1_501);
        assert!(cut.ends_with('…'));
        assert!(cut.starts_with(&"é".repeat(1_500)));
    }
}
```

- [ ] **Step 6: Wire the controller**

In `crates/app/src/tray/mod.rs`:

```rust
mod child;
mod controller;
pub mod panel;
pub mod status_item;
mod update_ui;

use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::MainThreadMarker;

use crate::update::release::Source;

/// Run the menu bar app. Never returns.
///
/// `source` is where updates are looked up; it is only used when this
/// process runs from a bundle.
pub fn run(source: Source) -> ! {
    let mtm = MainThreadMarker::new().expect("tray must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    // Accessory: menu bar only, no Dock icon, no menu bar menus of its own.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    // Held for the lifetime of the run loop: the status item, the menu
    // delegate and the notification centre all reference it unretained.
    let controller = controller::Controller::new(mtm, source);
    // The delegate is what receives the Finder double-click of an already
    // running LSUIElement app; without it that click is silently swallowed.
    app.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
    app.run();
    unreachable!("NSApplication::run does not return")
}
```

In `crates/app/src/main.rs`, replace the `None` arm:

```rust
        None => match cli::update_source(args.update_source) {
            Ok(source) => tray::run(source),
            Err(e) => Err(e),
        },
```

In `crates/app/src/tray/controller.rs`:

1. Imports: add `use super::update_ui::{self, UpdateIvars};` next to the other `super::` imports.
2. `Ivars`: make `status_item` `pub(super)`, and add as the last field:
   ```rust
       /// Everything about updates — see `update_ui`.
       pub(super) update: UpdateIvars,
   ```
3. Inside `define_class!`'s `impl Controller { … }`, after `power_changed_on_main`:
   ```rust
           #[unsafe(method(updateTimerFired:))]
           fn update_timer_fired(&self, _timer: *mut NSTimer) {
               self.start_check(false);
           }

           #[unsafe(method(checkForUpdates:))]
           fn check_for_updates(&self, _sender: *mut AnyObject) {
               self.start_check(true);
           }

           #[unsafe(method(installUpdate:))]
           fn install_update_action(&self, _sender: *mut AnyObject) {
               self.install_update();
           }

           #[unsafe(method(updateCheckDone:))]
           fn update_check_done(&self, _n: *mut NSNotification) {
               // Posted from the check thread; hop exactly like `powerChanged:`.
               //
               // SAFETY: `self` responds to `updateCheckDoneOnMain`, which takes
               // no argument, so the object passed is null.
               unsafe {
                   let _: () = msg_send![
                       self,
                       performSelectorOnMainThread: sel!(updateCheckDoneOnMain),
                       withObject: std::ptr::null_mut::<AnyObject>(),
                       waitUntilDone: false,
                   ];
               }
           }

           #[unsafe(method(updateCheckDoneOnMain))]
           fn update_check_done_on_main(&self) {
               self.check_finished();
           }
   ```
4. `menuWillOpen:`: add `self.refresh_update_items();` as its first line (before `mutate_state`).
5. `new`: signature `pub fn new(mtm: MainThreadMarker, source: Source) -> Retained<Self>` (import `crate::update::release::Source`); build `update: UpdateIvars::new(mtm, source),` in the `Ivars` literal; after `menu.addItem(&dashboard);` and before the `quit` item, add `this.install_update_items(&menu);`; after `this.sync_timer();` add `this.schedule_update_checks();`.
6. `observe_notifications`: inside the existing `unsafe` block, add
   ```rust
               default.addObserver_selector_name_object(
                   self,
                   sel!(updateCheckDone:),
                   Some(&NSString::from_str(update_ui::CHECK_DONE)),
                   None,
               );
   ```
   and extend the doc comment: "`updateCheckDone:` is posted by the check thread, so like `powerChanged:` it only hops."

- [ ] **Step 7: Build, test, and check the non-bundled path by hand**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets --examples -- -D warnings 2>&1 | tail -2
cargo test -p vitals 2>&1 | grep -E "^test result|FAILED|panicked" | head
```

Expected: clippy clean; every `test result: ok`.

Then start a tray from the build directory — not a bundle, so the menu must be unchanged — and stop it again:

```bash
cargo build -p vitals 2>&1 | tail -1
./target/debug/vitals & TRAY=$!; sleep 3; kill $TRAY
```

Expected: a `…` then digits appear in the menu bar for three seconds; nothing is printed. (Opening the menu by hand shows only Open Dashboard and Quit — the bundled path is verified in Task 13.)

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/update/checker.rs crates/app/src/update/mod.rs crates/app/src/tray/update_ui.rs crates/app/src/tray/mod.rs crates/app/src/tray/controller.rs crates/app/src/cli.rs crates/app/src/main.rs crates/app/tests/update_source.rs
git commit -m "feat(tray): check for updates 30 s after start, daily, and on demand

checker::check_now fetches releases/latest and compares; UpdateState
folds the outcome in (a newer latest replaces, an equal one clears, a
failure keeps). The tray schedules a one-shot and a daily NSTimer with an
hour of tolerance, runs the check on a worker thread, and hops back to
the main thread through a notification the way powerChanged: does. Check
for Updates… reports in an alert; automatic failures are one stderr line.
All of it only from a bundle. The hidden --update-source flag accepts a
loopback base for the manual end-to-end test and refuses anything else
before AppKit starts.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: The indicators — menu bar badge and the accent-coloured row

Section §3 in full, and §5 "In `status_item.rs`: the `(title, badge)` change detection".

**Files:**
- Modify: `crates/app/src/tray/status_item.rs`
- Modify: `crates/app/src/tray/update_ui.rs`
- Modify: `crates/app/src/tray/controller.rs`

**Interfaces:**
- Produces: `status_item::UPDATE_BADGE: &str = " ●"`; `status_item::TitleState { pub title: String, pub badge: bool }` (`PartialEq`) with `fn render(&self) -> (String, Option<(usize, usize)>)` (UTF-16 location and length of the badge); `update_ui::styled(text: &str, font: &NSFont, accent: (usize, usize)) -> Retained<NSMutableAttributedString>`; `Controller::refresh_title()`.

Binding facts (verified): `NSMutableAttributedString::from_nsstring(&NSString)`; `addAttribute_value_range(&self, &NSAttributedStringKey, &AnyObject, NSRange)` is `unsafe`; `NSFontAttributeName` and `NSForegroundColorAttributeName` are `extern static`s in `objc2-app-kit` (reading them is `unsafe`); `NSRange::new(location, length)`; `NSColor::labelColor()`, `NSColor::controlAccentColor()`, `NSFont::menuFontOfSize(0.0)`; `NSButton::setAttributedTitle(&NSAttributedString)` (an `NSMutableAttributedString` derefs to it); `NSControl::font() -> Option<Retained<NSFont>>`; `NSMenuItem::setAttributedTitle(Option<&NSAttributedString>)`.

- [ ] **Step 1: Write the failing tests in `status_item.rs`**

Append inside the existing `mod tests`:

```rust
    #[test]
    fn the_badge_alone_is_a_change() {
        let plain = TitleState { title: "12% · 17.8G".into(), badge: false };
        let badged = TitleState { title: "12% · 17.8G".into(), badge: true };
        assert_ne!(plain, badged, "a badge appearing must count as a redraw");
        assert_eq!(plain, TitleState { title: "12% · 17.8G".into(), badge: false });
    }

    #[test]
    fn a_badged_title_ends_in_the_badge_with_its_utf16_range() {
        let (text, range) = TitleState { title: "12% · 17.8G".into(), badge: true }.render();
        assert_eq!(text, "12% · 17.8G ●");
        // `·` and `●` are one UTF-16 unit each, so the badge starts at 11
        // and is two units long (the space and the dot).
        assert_eq!(range, Some((11, 2)));
        assert_eq!(UPDATE_BADGE.encode_utf16().count(), 2);
    }

    #[test]
    fn an_unbadged_title_renders_bare() {
        let (text, range) = TitleState { title: "⚠".into(), badge: false }.render();
        assert_eq!(text, "⚠");
        assert_eq!(range, None);
    }
```

- [ ] **Step 2: Add the types above the tests**

```rust
/// Appended to the title while an update is available: a space, then
/// U+25CF, drawn in the accent colour by the controller.
pub const UPDATE_BADGE: &str = " ●";

/// What the status item shows: the digits, and whether the badge follows
/// them. Two states that differ only in the badge are different titles,
/// so a badge appearing or disappearing is redrawn like any other change
/// — and nothing else is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleState {
    pub title: String,
    pub badge: bool,
}

impl TitleState {
    /// The full string, and the badge's range within it as `NSRange`
    /// counts: UTF-16 code units, not bytes or chars.
    pub fn render(&self) -> (String, Option<(usize, usize)>) {
        if !self.badge {
            return (self.title.clone(), None);
        }
        let location = self.title.encode_utf16().count();
        let length = UPDATE_BADGE.encode_utf16().count();
        (format!("{}{UPDATE_BADGE}", self.title), Some((location, length)))
    }
}
```

```bash
cargo test -p vitals --lib tray::status_item 2>&1 | tail -3
```

Expected: `test result: ok. 7 passed`.

- [ ] **Step 3: The attributed-string helper in `update_ui.rs`**

Add to the imports: `use objc2_app_kit::{NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName};` (merge into the existing `objc2_app_kit` line) and `NSMutableAttributedString, NSRange` into the `objc2_foundation` line. Add the function after `open_page`:

```rust
/// `text` in `font` and the label colour, with the accent colour over
/// `accent` (UTF-16 location and length). The label colour is set
/// explicitly because an attributed title otherwise draws in black, which
/// a dark menu bar or menu does not tolerate; the accent colour is added
/// last so it wins over the label colour on its range.
pub(super) fn styled(
    text: &str,
    font: &NSFont,
    accent: (usize, usize),
) -> Retained<NSMutableAttributedString> {
    let attributed = NSMutableAttributedString::from_nsstring(&NSString::from_str(text));
    let whole = NSRange::new(0, text.encode_utf16().count());
    let (location, length) = accent;
    // SAFETY: the keys are AppKit's own constants; the values are an
    // NSFont and NSColors, the types those keys take; both ranges lie
    // within the string, whose length is measured the way NSRange counts.
    unsafe {
        attributed.addAttribute_value_range(NSFontAttributeName, font, whole);
        attributed.addAttribute_value_range(NSForegroundColorAttributeName, &NSColor::labelColor(), whole);
        attributed.addAttribute_value_range(
            NSForegroundColorAttributeName,
            &NSColor::controlAccentColor(),
            NSRange::new(location, length),
        );
    }
    attributed
}
```

- [ ] **Step 4: The row's accent dot**

In `refresh_update_items`, replace the plain `setTitle` of the row:

```rust
                let title = format!("● Update to Vitals {}…", release.version);
                items.row.setAttributedTitle(Some(&styled(&title, &NSFont::menuFontOfSize(0.0), (0, 1))));
                items.row.setEnabled(true);
```

- [ ] **Step 5: The badge on the status item**

In `controller.rs`:

1. Import: `use super::status_item::{format_title, TitleState};`.
2. `Ivars.last_title` becomes `pub(super) last_title: RefCell<TitleState>`, its doc comment "Last title handed to AppKit, badge included, so an unchanged tick costs nothing."; initialise it with `RefCell::new(TitleState { title: PLACEHOLDER_TITLE.to_string(), badge: false })`.
3. Replace `set_title`:

```rust
    fn set_title(&self, title: &str) {
        let next = TitleState {
            title: title.to_string(),
            badge: self.ivars().update.state.borrow().available.is_some(),
        };
        if *self.ivars().last_title.borrow() == next {
            return; // budget rule: only touch AppKit when the string changed
        }
        let mtm = MainThreadMarker::from(self);
        if let Some(button) = self.ivars().status_item.button(mtm) {
            let (text, badge) = next.render();
            match (badge, button.font()) {
                (Some(range), Some(font)) => {
                    button.setAttributedTitle(&update_ui::styled(&text, &font, range))
                }
                _ => button.setTitle(&NSString::from_str(&text)),
            }
        }
        *self.ivars().last_title.borrow_mut() = next;
    }

    /// Redraw for a badge change alone: the digits are unchanged, so this
    /// is a no-op unless `available` moved.
    pub(super) fn refresh_title(&self) {
        let title = self.ivars().last_title.borrow().title.clone();
        self.set_title(&title);
    }
```

4. In `update_ui.rs`'s `check_finished`, after the `apply(...)` line, add `self.refresh_title();`.

- [ ] **Step 6: Build, lint, test**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets --examples -- -D warnings 2>&1 | tail -2
cargo test -p vitals 2>&1 | grep -E "^test result|FAILED" | head
```

Expected: clean; all ok. The rendering itself is checked in Task 13 (light and dark menu bars).

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/tray/status_item.rs crates/app/src/tray/update_ui.rs crates/app/src/tray/controller.rs
git commit -m "feat(tray): accent dot in the menu bar and an Update row when a release is newer

The status item's change detection now compares (title, badge), so the
dot appears and disappears with exactly one redraw and no extra ones.
The badge is an attributed title: the digits in the label colour and the
same font as before, the dot in controlAccentColor, ranges in UTF-16
units. The dropdown's top row carries the same dot.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: Install and relaunch from the alert

Section §4 "The alert" (three buttons), "Install" step 7 (terminate), "Failure" (the alert, the row), §3 item 1's `Installing…` state.

**Files:**
- Modify: `crates/app/src/tray/update_ui.rs`
- Modify: `crates/app/src/tray/controller.rs`

**Interfaces:**
- Consumes: `install::run(&Source, &Release, &Path) -> Result<(), UpdateError>` (Task 6), `app_url()` (Task 8).
- Produces: `update_ui::INSTALL_DONE`; `UpdateIvars.install_slot: Slot<Result<(), String>>`; on `Controller`: `start_install(Release)`, `install_finished()`.

- [ ] **Step 1: The slot and the notification**

In `update_ui.rs`:

```rust
/// Posted from the install thread once its result is in the slot.
pub(super) const INSTALL_DONE: &str = "com.billsun.vitals.updateInstallDone";
```

Add to `UpdateIvars`: `pub(super) install_slot: Slot<Result<(), String>>,` initialised with `Arc::new(Mutex::new(None))` in `new`.

- [ ] **Step 2: The three-button alert**

Replace `offer_install` and add the two functions after it. Add `NSAlertThirdButtonReturn` and `NSApplication` to the `objc2_app_kit` import, and `use crate::update::install;`.

```rust
    /// The release alert: *Install and Relaunch* (default), *Later*,
    /// *View Release*. *Later* changes nothing; the dot stays.
    pub(super) fn offer_install(&self, release: &Release) {
        let mtm = MainThreadMarker::from(self);
        let alert = NSAlert::new(mtm);
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str(&format!("Vitals {} is available", release.version)));
        alert.setInformativeText(&NSString::from_str(&truncate_notes(&release.notes)));
        alert.addButtonWithTitle(&NSString::from_str("Install and Relaunch"));
        alert.addButtonWithTitle(&NSString::from_str("Later"));
        alert.addButtonWithTitle(&NSString::from_str("View Release"));
        let response = alert.runModal();
        if response == NSAlertFirstButtonReturn {
            self.start_install(release.clone());
        } else if response == NSAlertThirdButtonReturn {
            open_page(&release.page_url);
        }
    }

    /// Run the install on a worker thread. Its result comes back through
    /// `INSTALL_DONE`. Nothing else may start while it runs: `start_check`
    /// refuses, and the row reads `Installing…`.
    pub(super) fn start_install(&self, release: Release) {
        let Some(source) = self.ivars().update.source.clone() else {
            return;
        };
        {
            let mut state = self.ivars().update.state.borrow_mut();
            if state.installing {
                return;
            }
            state.installing = true;
        }
        let app_url = app_url();
        let slot = Arc::clone(&self.ivars().update.install_slot);
        std::thread::Builder::new()
            .name("vitals-update-install".into())
            .spawn(move || {
                let result = install::run(&source, &release, &app_url).map_err(|e| e.to_string());
                *slot.lock().unwrap() = Some(result);
                post(INSTALL_DONE);
            })
            .expect("spawning the install thread");
    }

    /// On the main thread, after `INSTALL_DONE`. Success means the new
    /// bundle is in place and the helper is waiting for this process to
    /// exit: terminate. Failure means nothing changed: say why.
    pub(super) fn install_finished(&self) {
        let Some(result) = self.ivars().update.install_slot.lock().unwrap().take() else {
            return;
        };
        let mtm = MainThreadMarker::from(self);
        let version = {
            let mut state = self.ivars().update.state.borrow_mut();
            state.installing = false;
            state.available.as_ref().map(|r| r.version.to_string()).unwrap_or_default()
        };
        match result {
            Ok(()) => NSApplication::sharedApplication(mtm).terminate(None),
            Err(reason) => alert(mtm, &format!("Couldn't install Vitals {version}"), &reason),
        }
    }
```

- [ ] **Step 3: The `Installing…` row**

In `refresh_update_items`, the `(Some(release), _)` arm becomes:

```rust
            (Some(release), _) => {
                if !present {
                    menu.insertItem_atIndex(&items.row_separator, 0);
                    menu.insertItem_atIndex(&items.row, 0);
                }
                if state.installing {
                    items.row.setAttributedTitle(None);
                    items.row.setTitle(&NSString::from_str("Installing…"));
                    items.row.setEnabled(false);
                } else {
                    let title = format!("● Update to Vitals {}…", release.version);
                    items.row.setAttributedTitle(Some(&styled(&title, &NSFont::menuFontOfSize(0.0), (0, 1))));
                    items.row.setEnabled(true);
                }
            }
```

- [ ] **Step 4: The selectors and the observer**

In `controller.rs`, inside `define_class!` after `update_check_done_on_main`:

```rust
        #[unsafe(method(updateInstallDone:))]
        fn update_install_done(&self, _n: *mut NSNotification) {
            // Posted from the install thread; hop exactly like `powerChanged:`.
            //
            // SAFETY: `self` responds to `updateInstallDoneOnMain`, which
            // takes no argument, so the object passed is null.
            unsafe {
                let _: () = msg_send![
                    self,
                    performSelectorOnMainThread: sel!(updateInstallDoneOnMain),
                    withObject: std::ptr::null_mut::<AnyObject>(),
                    waitUntilDone: false,
                ];
            }
        }

        #[unsafe(method(updateInstallDoneOnMain))]
        fn update_install_done_on_main(&self) {
            self.install_finished();
        }
```

and in `observe_notifications`, next to the `updateCheckDone:` registration:

```rust
            default.addObserver_selector_name_object(
                self,
                sel!(updateInstallDone:),
                Some(&NSString::from_str(update_ui::INSTALL_DONE)),
                None,
            );
```

- [ ] **Step 5: Build, lint, test**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets --examples -- -D warnings 2>&1 | tail -2
cargo test -p vitals 2>&1 | grep -E "^test result|FAILED" | head
```

Expected: clean; all ok. The flow itself — dot, row, alert, install, relaunch, and the `BadSignature` refusal — is the manual test in Task 13; it needs two signed bundles and `/Applications`, which this task must not touch.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/tray/update_ui.rs crates/app/src/tray/controller.rs
git commit -m "feat(tray): Install and Relaunch

The release alert gains its default button. Install runs on a worker
thread — stage, download, verify, unpack, check the version and the
signature, spawn the relaunch helper, swap — and reports through a
notification the controller hops onto the main thread for: on success
the app terminates and the helper opens the new bundle; on failure the
installed copy is untouched, one alert says why, and the row returns to
offering the update. The row reads Installing… meanwhile, and no check
starts.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 11: Start at Login, and the install targets that no longer need a LaunchAgent

Section §6 in full ("The app registers itself", "Dropdown", "The LaunchAgent goes away") and §5 "Login item".

**Files:**
- Create: `crates/app/src/tray/login_item.rs`
- Modify: `crates/app/src/tray/mod.rs`
- Modify: `crates/app/src/tray/update_ui.rs`
- Modify: `crates/app/src/tray/controller.rs`
- Modify: `crates/app/src/cli.rs`
- Modify: `crates/app/src/main.rs`
- Modify: `Makefile`
- Delete: `resources/com.billsun.vitals.plist`

**Interfaces:**
- Produces: `tray::login_item::{LoginStatus, status(), set(bool) -> Result<(), String>, open_settings(), register_once(), menu_state(LoginStatus) -> (&'static str, bool, bool), run(LoginItemAction) -> Result<(), String>}`; `cli::LoginItemAction { On, Off, Status }`; `Command::LoginItem { action }` (`vitals login-item on|off|status`); `UpdateItems.login`; `Controller::{refresh_login_item, toggle_login_item}`.

Binding facts (verified in `objc2-service-management 0.3.2`): all `unsafe` — `SMAppService::mainAppService() -> Retained<SMAppService>`, `registerAndReturnError(&self) -> Result<(), Retained<NSError>>`, `unregisterAndReturnError(&self)`, `status(&self) -> SMAppServiceStatus`, `SMAppService::openSystemSettingsLoginItems()`. `SMAppServiceStatus` is a struct with `NotRegistered = 0`, `Enabled = 1`, `RequiresApproval = 2`, `NotFound = 3`, comparable with `==`. `NSUserDefaults::standardUserDefaults()`, `boolForKey(&NSString)`, `setBool_forKey(bool, &NSString)` are safe. `NSControlStateValueOn` / `NSControlStateValueOff` are plain `pub static`s in `objc2-app-kit`.

- [ ] **Step 1: Write the failing tests**

Create `crates/app/src/tray/login_item.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_state_covers_every_status() {
        assert_eq!(menu_state(LoginStatus::Enabled), ("Start at Login", true, true));
        assert_eq!(menu_state(LoginStatus::NotRegistered), ("Start at Login", false, true));
        assert_eq!(
            menu_state(LoginStatus::RequiresApproval),
            ("Start at Login — approve in System Settings…", false, true)
        );
        assert_eq!(menu_state(LoginStatus::NotFound), ("Start at Login", false, false));
    }
}
```

And in `crates/app/src/cli.rs`'s `mod tests`:

```rust
    #[test]
    fn login_item_is_a_hidden_verb_with_three_actions() {
        use clap::CommandFactory;
        for (word, action) in [
            ("on", LoginItemAction::On),
            ("off", LoginItemAction::Off),
            ("status", LoginItemAction::Status),
        ] {
            let cli = Cli::try_parse_from(["vitals", "login-item", word]).unwrap();
            assert!(matches!(cli.command, Some(Command::LoginItem { action: a }) if a == action));
        }
        assert!(Cli::try_parse_from(["vitals", "login-item"]).is_err());
        assert!(!Cli::command().render_help().to_string().contains("login-item"));
    }
```

- [ ] **Step 2: Run them to see them fail**

```bash
cargo test -p vitals --lib login_item 2>&1 | grep -E "^error" | head -3
```

Expected: compile errors — `login_item` is not a module, `LoginItemAction` does not exist.

- [ ] **Step 3: The verb**

In `crates/app/src/cli.rs`, after the `Relaunch` variant:

```rust
    /// Manage this bundle's Start at Login registration.
    ///
    /// Used by `make uninstall-app` and the manual test in CONTRIBUTING.md;
    /// hidden from `--help`. Only meaningful when run from the installed
    /// bundle's own binary.
    #[command(hide = true)]
    LoginItem {
        #[command(subcommand)]
        action: LoginItemAction,
    },
```

and, after the `Command` enum:

```rust
#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginItemAction {
    /// Register the bundle as a login item.
    On,
    /// Remove the registration.
    Off,
    /// Print `enabled`, `not-registered`, `requires-approval` or `not-found`.
    Status,
}
```

In `crates/app/src/main.rs`, add the arm before `None`:

```rust
        Some(Command::LoginItem { action }) => tray::login_item::run(action),
```

- [ ] **Step 4: Write the module above the tests**

```rust
//! Start at Login through `SMAppService`, the API a bundled app uses to
//! register itself as a login item (macOS 13+; our floor is 14). No
//! LaunchAgent plist, no helper: the registration names the bundle, so it
//! survives the updater swapping the bundle and goes away with the app.
//!
//! `menu_state` is the pure part and is unit-tested. The calls into
//! ServiceManagement are verified by running them — the manual test in
//! CONTRIBUTING.md — because there is nothing to assert on outside a
//! bundle.

use objc2_foundation::{NSString, NSUserDefaults};
use objc2_service_management::{SMAppService, SMAppServiceStatus};

use crate::cli::LoginItemAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginStatus {
    Enabled,
    NotRegistered,
    /// Registered, but macOS wants the user to approve it in System
    /// Settings before it takes effect.
    RequiresApproval,
    /// Not a bundle, or the bundle cannot be a login item.
    NotFound,
}

/// The `NSUserDefaults` key that says `register_once` has run.
const REGISTERED_KEY: &str = "registeredAtLogin";

pub fn status() -> LoginStatus {
    // SAFETY: `mainAppService` and `status` have no preconditions; outside
    // a bundle they report `NotFound`.
    let raw = unsafe { SMAppService::mainAppService().status() };
    if raw == SMAppServiceStatus::Enabled {
        LoginStatus::Enabled
    } else if raw == SMAppServiceStatus::RequiresApproval {
        LoginStatus::RequiresApproval
    } else if raw == SMAppServiceStatus::NotFound {
        LoginStatus::NotFound
    } else {
        LoginStatus::NotRegistered
    }
}

/// Register (`true`) or unregister (`false`) this bundle. Registering an
/// already-registered bundle succeeds.
pub fn set(on: bool) -> Result<(), String> {
    // SAFETY: no preconditions; a failure comes back as the error.
    let result = unsafe {
        let service = SMAppService::mainAppService();
        if on {
            service.registerAndReturnError()
        } else {
            service.unregisterAndReturnError()
        }
    };
    result.map_err(|e| e.localizedDescription().to_string())
}

/// Open System Settings at Login Items, for the `RequiresApproval` state.
pub fn open_settings() {
    // SAFETY: no preconditions.
    unsafe { SMAppService::openSystemSettingsLoginItems() }
}

/// First bundled launch only: register, and remember having done so, so
/// a user who later turns it off is not enrolled again next launch.
pub fn register_once() {
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = NSString::from_str(REGISTERED_KEY);
    if defaults.boolForKey(&key) {
        return;
    }
    match set(true) {
        Ok(()) => defaults.setBool_forKey(true, &key),
        Err(e) => eprintln!("vitals: could not register as a login item: {e}"),
    }
}

/// Pure: `(title, checked, enabled)` for the *Start at Login* menu item.
pub fn menu_state(status: LoginStatus) -> (&'static str, bool, bool) {
    match status {
        LoginStatus::Enabled => ("Start at Login", true, true),
        LoginStatus::NotRegistered => ("Start at Login", false, true),
        LoginStatus::RequiresApproval => ("Start at Login — approve in System Settings…", false, true),
        LoginStatus::NotFound => ("Start at Login", false, false),
    }
}

/// The hidden `vitals login-item` verb.
pub fn run(action: LoginItemAction) -> Result<(), String> {
    match action {
        LoginItemAction::On => set(true),
        LoginItemAction::Off => set(false),
        LoginItemAction::Status => {
            let word = match status() {
                LoginStatus::Enabled => "enabled",
                LoginStatus::NotRegistered => "not-registered",
                LoginStatus::RequiresApproval => "requires-approval",
                LoginStatus::NotFound => "not-found",
            };
            println!("{word}");
            Ok(())
        }
    }
}
```

- [ ] **Step 5: The menu item and the first-launch registration**

In `crates/app/src/tray/mod.rs`: add `pub mod login_item;` (between `child` and `panel`, keeping the list alphabetical), and in `run`, after `let controller = controller::Controller::new(mtm, source);`:

```rust
    // A bundled launch enrols itself as a login item, once; the status item
    // already exists, so a slow ServiceManagement call cannot delay it.
    if update_ui::is_bundled() {
        login_item::register_once();
    }
```

In `crates/app/src/tray/update_ui.rs`:

1. Imports: add `NSControlStateValueOff, NSControlStateValueOn` to the `objc2_app_kit` line, and `use super::login_item::{self, LoginStatus};`.
2. `UpdateItems` gains, between `row_separator` and `check`:
   ```rust
       /// `Start at Login`, refreshed from the registration on every open.
       pub(super) login: Retained<NSMenuItem>,
   ```
   and `UpdateIvars::new` creates it: `let login = NSMenuItem::new(mtm); login.setTitle(&NSString::from_str("Start at Login"));` and puts `login` in the struct literal.
3. `install_update_items`: target `login` at `toggleLoginItem:` inside the same `unsafe` block, and add `menu.addItem(&items.login);` *before* `menu.addItem(&items.check);`.
4. Two more methods on `Controller`:

```rust
    /// The *Start at Login* row, from the live registration.
    pub(super) fn refresh_login_item(&self) {
        let Some(items) = self.ivars().update.items.as_ref() else {
            return;
        };
        let (title, checked, enabled) = login_item::menu_state(login_item::status());
        items.login.setTitle(&NSString::from_str(title));
        items.login.setState(if checked { NSControlStateValueOn } else { NSControlStateValueOff });
        items.login.setEnabled(enabled);
    }

    /// The row's action: flip the registration, or open the Settings
    /// pane when macOS is waiting for approval.
    pub(super) fn toggle_login_item(&self) {
        let result = match login_item::status() {
            LoginStatus::Enabled => login_item::set(false),
            LoginStatus::NotRegistered => login_item::set(true),
            LoginStatus::RequiresApproval => {
                login_item::open_settings();
                Ok(())
            }
            LoginStatus::NotFound => Ok(()),
        };
        if let Err(reason) = result {
            alert(MainThreadMarker::from(self), "Couldn't change Start at Login", &reason);
        }
    }
```

In `crates/app/src/tray/controller.rs`: the selector, after `install_update_action`:

```rust
        #[unsafe(method(toggleLoginItem:))]
        fn toggle_login_item_action(&self, _sender: *mut AnyObject) {
            self.toggle_login_item();
        }
```

and in `menuWillOpen:`, after `self.refresh_update_items();`, add `self.refresh_login_item();`.

- [ ] **Step 6: The Makefile**

Replace the first two lines (`BIN := target/release/vitals` and `PREFIX ?= /usr/local`; the `.PHONY` line below them stays) with:

```make
BIN := target/release/vitals
# Where the `vitals` symlink goes. $(HOME)/.local/bin needs no sudo;
# `make install PREFIX=/usr/local` if you'd rather have it there.
PREFIX ?= $(HOME)/.local
APP := /Applications/Vitals.app
# Left behind by installs older than the login-item registration.
LAUNCH_AGENT := $(HOME)/Library/LaunchAgents/com.billsun.vitals.plist
```

Replace the `bundle` comment, `install-app` and `uninstall-app` targets (everything from the line `# Packages target/release/vitals into dist/Vitals.app: an ad-hoc-signed,` to the end of the file) with:

```make
# Packages target/release/vitals into dist/Vitals.app: an LSUIElement app
# bundle (no Dock icon, no app switcher entry), ad-hoc signed unless
# SIGN_IDENTITY names a codesign identity. See scripts/bundle.sh.
bundle: build
	./scripts/bundle.sh

# Installs the bundle to /Applications, symlinks $(PREFIX)/bin/vitals to
# the bundled binary, and opens the app, which registers itself as a login
# item on its first bundled launch (crates/app/src/tray/login_item.rs). A
# running tray is quit first, or `open` would only re-front the old one. A
# LaunchAgent left by an older install is unloaded and removed so the tray
# is not started twice at login.
install-app: bundle
	install -d "$(PREFIX)/bin"
	-killall vitals 2>/dev/null
	rm -rf "$(APP)"
	cp -R dist/Vitals.app /Applications/
	ln -sf "$(APP)/Contents/MacOS/vitals" "$(PREFIX)/bin/vitals"
	if [ -f "$(LAUNCH_AGENT)" ]; then launchctl unload "$(LAUNCH_AGENT)" 2>/dev/null; rm -f "$(LAUNCH_AGENT)"; fi
	open "$(APP)"
	@echo "installed $(APP); $(PREFIX)/bin/vitals -> $(APP)/Contents/MacOS/vitals"

# Turns the login item off from the installed bundle (the registration
# names that bundle, so its own binary must do it), quits the tray, and
# removes the bundle and the symlink.
uninstall-app:
	-"$(APP)/Contents/MacOS/vitals" login-item off 2>/dev/null
	-killall vitals 2>/dev/null
	rm -rf "$(APP)"
	rm -f "$(PREFIX)/bin/vitals"
```

Then delete the LaunchAgent plist:

```bash
git rm -q resources/com.billsun.vitals.plist
grep -rn "com.billsun.vitals.plist\|LaunchAgents" Makefile scripts resources crates || echo "no references left in code"
```

Expected: `no references left in code` (README and CONTRIBUTING still mention it; Task 12 rewrites them).

- [ ] **Step 7: Build, lint, test, dry-run the Makefile**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets --examples -- -D warnings 2>&1 | tail -2
cargo test -p vitals 2>&1 | grep -E "^test result|FAILED" | head
make -n install-app | tail -8
make -n uninstall-app
cargo run -q -p vitals -- --help | grep -c "login-item\|relaunch"
```

Expected: clean; all ok; `make -n` prints the commands above with `$(HOME)/.local` expanded and touches nothing (`-n` is a dry run — **do not run these targets without `-n`**); the last line prints `0`.

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/tray/login_item.rs crates/app/src/tray/mod.rs crates/app/src/tray/update_ui.rs crates/app/src/tray/controller.rs crates/app/src/cli.rs crates/app/src/main.rs Makefile resources/com.billsun.vitals.plist
git commit -m "feat(tray): Start at Login via SMAppService; the LaunchAgent is gone

The bundled app registers itself as a login item on its first launch and
remembers having done so, so turning it off sticks. Start at Login in the
dropdown reflects the live registration and flips it, or opens System
Settings when macOS wants approval. The hidden \`vitals login-item\`
verb exists for make uninstall-app and the manual test. make install-app
now copies, symlinks (PREFIX defaults to ~/.local: no sudo), removes a
LaunchAgent from an older install, and opens the app; the plist and the
launchctl steps are deleted.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 12: Documentation

Spec "Documentation changes" and §6 "README 'Install', rewritten". `docs/budget.md` is Task 13's, because its content is measurements.

**Files:**
- Modify: `README.md`
- Modify: `CONTRIBUTING.md`
- Modify: `SECURITY.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: README — Install**

Replace everything from `## Install` up to (not including) the next `## ` heading with:

````markdown
## Install

### Download

Requirements: an Apple Silicon Mac running macOS 14 or newer.

1. Open the [latest release](https://github.com/billsun9305/vitals/releases/latest)
   and download `Vitals-<version>.dmg`.
2. Open it and drag **Vitals** to **Applications**.
3. Open Vitals from Applications. It is notarized, so it opens without a
   dialog, appears in the menu bar, and starts at login from now on — the
   *Start at Login* item in its menu turns that off.

When a newer version is published, the menu bar item shows an
accent-coloured dot and *Update to Vitals x.y.z…* at the top of its menu
installs it and relaunches. [Updates](#updates) below says exactly what is
checked, when, and what is sent.

If a release's notes say the build is not notarized, macOS refuses it the
first time: go to System Settings → Privacy & Security, scroll to the
message about Vitals, and click *Open Anyway*.

To use the `vitals` CLI alongside the app, symlink the bundled binary onto
your `PATH`:

```bash
ln -s /Applications/Vitals.app/Contents/MacOS/vitals ~/.local/bin/vitals
```

### From source, menu bar app

You need `git`, Rust via [rustup](https://rustup.rs) (`rust-toolchain.toml`
picks the toolchain) and Node 22 or newer.

```bash
git clone https://github.com/billsun9305/vitals.git
cd vitals
make install-app    # builds, bundles, copies to /Applications, opens it
```

This copies `dist/Vitals.app` to `/Applications/Vitals.app`, symlinks
`~/.local/bin/vitals` to the bundled binary (`PREFIX=/usr/local` to put it
there instead) and opens the app, which registers itself as a login item
on its first launch. Nothing needs `sudo`. `make uninstall-app` turns the
login item off, quits the tray, and removes the bundle and the symlink.

A source build is ad-hoc signed: it runs, and it updates itself, but the
updater cannot check a release's signature against it — see
[SECURITY.md](SECURITY.md). To sign a local build with your own Developer
ID identity: `SIGN_IDENTITY="Developer ID Application: …" make install-app`.

### From source, CLI only

```bash
make install    # cargo build --release, then symlink ~/.local/bin/vitals
```

`make install PREFIX=/usr/local` puts the symlink there instead (that one
needs write access to `/usr/local/bin`). `make uninstall` removes it.

````

- [ ] **Step 2: README — the menu bar section**

Replace the two paragraphs that begin `` `make install-app` packages this into `dist/Vitals.app` `` and `A LaunchAgent rather than `SMAppService`:` with:

```markdown
### Start at Login

The app registers itself as a login item the first time it runs from a
bundle, through `SMAppService` — the registration System Settings →
General → Login Items shows, and nothing else: no LaunchAgent, no helper.
*Start at Login* in the dropdown turns it off and on. The registration
names the bundle rather than a path inside it, so it survives the updater
replacing the bundle. If macOS asks you to approve the item, the menu
item says so and opens the right Settings pane.

### Updates

Thirty seconds after the tray starts, and then once a day (with an hour
of slack so macOS can batch the wake-up with others), it asks
`api.github.com/repos/billsun9305/vitals/releases/latest` for the newest
version. The request carries the GitHub API headers and
`User-Agent: vitals/<version>` — nothing that identifies you or your
machine — and nothing is downloaded until you ask. *Check for Updates…*
runs the same check on demand and reports either way.

When the latest release is newer, an accent-coloured dot appears after the
digits in the menu bar and *Update to Vitals x.y.z…* appears at the top of
the dropdown. *Install and Relaunch* downloads the release's tarball and
`SHA256SUMS`, verifies the hash, unpacks next to the bundle, checks that
the new bundle's version is the release's and — when the running copy is
signed with a Developer ID — that the new one is validly signed by the
same team, then swaps it in and relaunches. Any failure leaves the
installed copy untouched and says why.
```

Also, in the intro paragraph near the top, the sentence that ends `see [below](#the-menu-bar-app) for what it does and how to
install it as a login item.` becomes `see [below](#the-menu-bar-app) for what it does, and [Install](#install) for the download.`

- [ ] **Step 3: CONTRIBUTING**

1. Replace the paragraph `No `sudo`, no entitlements, no signing identity — the app bundle is ad-hoc signed and only ever installed on your own machine.` with:

   ```markdown
   No `sudo` and no entitlements. A local build is ad-hoc signed;
   `SIGN_IDENTITY="Developer ID Application: …" make bundle` signs with an
   identity of your own, which is how the release workflow builds — see
   *Signing, updates and the hidden verbs* below.
   ```

2. In the "Where things live" table, replace the `resources/` and `scripts/` rows:

   ```markdown
   | `resources/` | `Info.plist` for the bundle. |
   | `scripts/` | `bundle.sh` (what `make bundle` runs); `release.sh` and `changelog-section.sh` (cutting a release, see README); `release-secrets.sh` (the maintainer's one-time setup of the signing secrets). |
   ```

3. Add this section after "Where things live", before "The rules that are not style":

   ````markdown
   ## Signing, updates and the hidden verbs

   `make bundle` signs `dist/Vitals.app` ad hoc unless `SIGN_IDENTITY`
   names a codesign identity, in which case it signs with the hardened
   runtime and a timestamp — what `.github/workflows/release.yml` does
   with the repository's Developer ID certificate. An ad-hoc build runs
   and updates itself; only a Developer-ID-signed build verifies a
   release's signature before installing it.

   Three subcommands are hidden from `--help` because nothing but the app
   itself should run them: `vitals window --url … --parent …` (the
   dashboard window process), `vitals relaunch --parent … --app …` (the
   updater's helper: waits for the tray to exit, then opens the new
   bundle) and `vitals login-item on|off|status` (used by `make
   uninstall-app` and the test below). The bare tray invocation also
   takes a hidden `--update-source http://127.0.0.1:<port>/`, which
   points the updater at a local server instead of GitHub and refuses
   anything that is not loopback.

   The updater and the login item are verified by running them. Before
   tagging a release that changes either:

   1. **Update, end to end.** Build the current version and bundle it;
      then bump `Cargo.toml` and `resources/Info.plist` to the next patch
      version, bundle again, and serve that bundle as a release:

      ```bash
      make bundle && cp -R dist/Vitals.app /tmp/old.app
      /usr/libexec/PlistBuddy -c 'Set :CFBundleShortVersionString 0.1.1' resources/Info.plist
      sed -i '' 's/^version = "0.1.0"/version = "0.1.1"/' Cargo.toml
      make bundle
      mkdir -p /tmp/serve/releases && cd /tmp/serve
      COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -C "$OLDPWD/dist" -czf Vitals-0.1.1-arm64.tar.gz Vitals.app
      shasum -a 256 Vitals-0.1.1-arm64.tar.gz > SHA256SUMS
      cat > releases/latest <<'JSON'
      {"tag_name":"v0.1.1","draft":false,"prerelease":false,"html_url":"http://127.0.0.1:8000/","body":"Test release.",
       "assets":[{"name":"Vitals-0.1.1-arm64.tar.gz","browser_download_url":"http://127.0.0.1:8000/Vitals-0.1.1-arm64.tar.gz"},
                 {"name":"SHA256SUMS","browser_download_url":"http://127.0.0.1:8000/SHA256SUMS"}]}
      JSON
      python3 -m http.server 8000 &
      cd "$OLDPWD" && git checkout -- Cargo.toml Cargo.lock resources/Info.plist
      ```

      Install `/tmp/old.app` as `/Applications/Vitals.app`, quit any
      running tray, and start it with
      `/Applications/Vitals.app/Contents/MacOS/vitals --update-source http://127.0.0.1:8000/`.
      Within a minute the dot appears; the dropdown's top row offers
      0.1.1; *Install and Relaunch* replaces the bundle and the new tray
      reports 0.1.1 as the latest. With both bundles signed by the same
      Developer ID the signature check passes; repeat with the 0.1.1
      bundle signed ad hoc and the install must refuse with a signature
      error, leaving 0.1.0 in place. Record the tray's footprint before
      and after a check in `docs/budget.md`.
   2. **Login item.** After `make install-app`, Vitals is listed under
      System Settings → General → Login Items, the dropdown's *Start at
      Login* is checked, unchecking it removes the entry, and
      `/Applications/Vitals.app/Contents/MacOS/vitals login-item status`
      agrees with each state.
   3. **Download.** After a tagged release, download the DMG with a
      browser, drag, open: no Gatekeeper dialog.
   ````

4. Rule 3 under "The rules that are not style" currently reads `**No `sudo`, no subprocesses, no `powermetrics`.** Everything is read in-process.` Replace it with:

   ```markdown
   3. **No `sudo`, no subprocesses, no `powermetrics`.** Everything is read
      in-process. The one subprocess in the codebase is the updater's
      `vitals relaunch` helper — our own binary, started so the new bundle
      can be opened after the tray exits — and the dashboard window, which
      is a second process by design (`docs/budget.md`).
   ```

5. In "Commits and pull requests", after the CI bullet, add:

   ```markdown
   - A tag `vX.Y.Z` runs `.github/workflows/release.yml`, which publishes
     a GitHub Release; cut one with `scripts/release.sh`, never by hand.
   ```

- [ ] **Step 4: SECURITY**

Add two bullets to the "What is and isn't in scope" list, after the "The binary runs unprivileged" bullet:

```markdown
- **Updates are verified twice.** The updater downloads only from
  `github.com/billsun9305/vitals/releases/download/`, checks the tarball's
  SHA-256 against the release's `SHA256SUMS`, and — when the running copy
  carries a Developer ID signature — requires the new bundle to be validly
  signed by the same Team ID before anything is replaced. A way to make an
  installed, signed copy accept a bundle signed by anyone else is a
  vulnerability. A copy built from source is ad-hoc signed and has no Team
  ID, so it gets the hash check only, which proves the tarball is the one
  the release published, not who published it.
- **The updater never executes anything it downloaded.** The new bundle is
  swapped in with a rename and started by a helper that is our own,
  already-running binary; no script, no installer package.
```

- [ ] **Step 5: CHANGELOG**

In the `### Added` list under `## [Unreleased]`: change the menu bar app bullet's last sentence from `Installs as a LaunchAgent via `make install-app`.` to `` `make install-app` installs it to `/Applications`. `` and append three bullets:

```markdown
- **Releases** — `scripts/release.sh` cuts a version; the tag builds,
  signs, notarizes and publishes `Vitals-<version>.dmg`, the updater's
  tarball and `SHA256SUMS` on GitHub Releases.
- **In-app updates** — the menu bar app checks GitHub Releases daily and
  on demand, shows an accent dot and an *Update to Vitals x.y.z…* row, and
  installs with one click after verifying the hash and, for a signed copy,
  the Developer ID signature.
- **Start at Login** — the app registers itself with `SMAppService` on its
  first launch, with a toggle in the menu. There is no LaunchAgent.
```

- [ ] **Step 6: Check the links and the leftovers**

```bash
grep -n "LaunchAgent\|launchctl\|sudo make\|will never leave this machine\|not notarized" README.md CONTRIBUTING.md SECURITY.md CHANGELOG.md
grep -n "^## \|^### " README.md | sed -n '1,40p'
```

Expected: the first grep finds only the deliberate mentions — README's *Start at Login* paragraph ("no LaunchAgent, no helper"), CHANGELOG's "There is no LaunchAgent", and README's "If a release's notes say the build is not notarized" — and nothing recommending `sudo`, `launchctl`, or a plist. The second shows `## Install` with the three `###` subsections, and `### Start at Login` and `### Updates` inside the menu bar section.

- [ ] **Step 7: Commit**

```bash
git add README.md CONTRIBUTING.md SECURITY.md CHANGELOG.md
git commit -m "docs: download install, Start at Login, Updates, signing and the manual tests

README's Install is three subsections — download the notarized DMG, from
source with the app, from source CLI only — with no sudo anywhere, and
the menu bar section explains the login item and exactly what the update
check sends and when. CONTRIBUTING documents SIGN_IDENTITY, the hidden
verbs, --update-source and the manual tests; SECURITY the two-layer
verification and what the hash alone proves on an ad-hoc copy; CHANGELOG
lists both features.

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 13: Manual verification and the budget numbers (coordinator-run)

Section §4 "Budget" and §5 "Manual, before the first tag". **This task is run by the coordinator, not a subagent:** it reinstalls `/Applications/Vitals.app`, restarts the tray and needs the human's screen. The signed variant needs the Developer ID certificate in the login keychain; if `security find-identity -v -p codesigning | grep -c "Developer ID Application"` prints `0`, run the ad-hoc variant, record that, and leave the signed run and the `BadSignature` case listed as pending in `docs/budget.md` for when the certificate exists.

**Files:**
- Modify: `docs/budget.md` (new section at the end)

- [ ] **Step 1: Binary size**

```bash
cd /Users/billsun/code/vitals && make bundle 2>&1 | tail -1
ls -l target/release/vitals | awk '{print $5}'
du -sk dist/Vitals.app | cut -f1
```

Record both against the 6 MB cap, next to the number the previous section of `docs/budget.md` recorded.

- [ ] **Step 2: The update, end to end**

Follow CONTRIBUTING's manual test 1 exactly (it is the procedure Task 12 wrote), with `SIGN_IDENTITY` set for both bundles when the certificate exists. Note the pid of the tray started with `--update-source` and measure:

```bash
PID=$(pgrep -f "Vitals.app/Contents/MacOS/vitals --update-source" | head -1)
footprint -p "$PID" 2>/dev/null | grep -E "phys_footprint|Physical footprint" | head -1
```

once before the first check (within 30 s of starting), once after the dot has appeared, and once a minute later. Then confirm, in order: the dot after the digits, in the accent colour, legible on both a light and a dark menu bar (switch the appearance in System Settings while it runs); *Update to Vitals 0.1.1…* as the top row with a separator under it; *Check for Updates…* → the alert with the three buttons; *Later* → nothing changes; *Install and Relaunch* → the row reads *Installing…* if the menu is opened during the download, the old tray disappears, the new one appears, and its *Check for Updates…* says *You're up to date — Vitals 0.1.1 is the latest*; `stat -f %m /Applications/Vitals.app` changed; no `com.apple.quarantine` on the new bundle (`xattr -l /Applications/Vitals.app`). When signed: repeat with the 0.1.1 bundle re-signed ad hoc (`codesign --force --deep --sign - dist/Vitals.app` before packing) and confirm the *Couldn't install Vitals 0.1.1* alert mentions the signature and 0.1.0 is untouched.

- [ ] **Step 3: The login item**

CONTRIBUTING's manual test 2, after `make install-app` of the real (unbumped) build.

- [ ] **Step 4: The hardened runtime**

Only when signed: from the Developer-ID-signed bundle, run the tray, open the dashboard window, and `vitals snapshot`, `top`, `pressure --human`, `watch -n 2` from `/Applications/Vitals.app/Contents/MacOS/vitals`. Nothing should differ from the ad-hoc build; if anything is refused, `log stream --predicate 'process == "vitals"' --style compact` names the entitlement.

- [ ] **Step 5: Record**

Append to `docs/budget.md`:

```markdown
## Updates, signing and the login item

Measured on <date>, <chip>, macOS <version>, <signed with Developer ID | ad-hoc: the certificate did not exist yet>.

| | Value | Cap |
|---|---|---|
| Stripped binary | <n> MB | 6 MB |
| `Vitals.app` on disk | <n> MB | — |
| Tray footprint before the first check | <n> MB | 25 MB |
| Tray footprint after a check found an update | <n> MB | 25 MB |
| Tray footprint one minute later | <n> MB | 25 MB |

The check is one `NSURLSession` created for the request and invalidated
after it; a minute later nothing of it remains, which is the third row.
Between checks the only thing that exists is one `NSTimer` a day with an
hour of tolerance.

Update end to end (`--update-source` against a local server): dot in <n> s,
row, alert, install and relaunch in <n> s; the new bundle carries no
quarantine attribute. <Signature check passed with both bundles signed by
Team ID 4WD8D5Y6NF, and refused an ad-hoc 0.1.1 with `BadSignature`,
leaving 0.1.0 in place. | Signature check skipped: the running copy was
ad-hoc; the signed run is pending the certificate.>

Login item: registered on first launch, shown in System Settings, toggled
off and on from the dropdown, `login-item status` agreeing each time.
```

with every `<…>` replaced by what was measured. Commit:

```bash
git add docs/budget.md
git commit -m "docs(budget): the updater's cost and the manual verification record

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Order of work, and what comes after

Tasks run in numeric order; each leaves `main`'s tests green and `make lint` clean. Tasks 1–12 are subagent tasks; Task 13 is the coordinator's. After Task 13, the maintainer creates the Developer ID certificate and the App Store Connect key, runs `scripts/release-secrets.sh`, and — when they say so — `scripts/release.sh 0.1.0` followed by `git push --follow-tags`, which produces the first Release the installed apps will later update from.
