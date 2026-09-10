# In-app updates — design

Date: 2026-09-09. Status: approved in conversation, awaiting written review.

## Goal

When a new version of vitals is published, every installed menu bar copy
shows that an update exists and can install it in one click, without the
user visiting GitHub, downloading anything by hand, or re-running
`make install-app`.

## Non-goals

- No automatic installs. The user always clicks *Install and Relaunch*.
- No "skip this version", no beta channel, no update settings UI.
- No code signing beyond the existing ad-hoc signature, no notarization.
- No updating of the LaunchAgent plist (it is not part of the bundle and
  nothing in it changes between versions today).
- Nothing for the CLI verbs. Updates are a tray feature.

## Decisions already made

| Question | Answer |
|---|---|
| Where releases come from | GitHub Releases for `billsun9305/vitals`, built by CI from a `v*` tag |
| How the user learns of one | An accent-coloured `●` after the digits in the menu bar, and a row at the top of the dropdown |
| What a click does | A confirmation alert with the release notes: *Install and Relaunch* / *Later* / *View Release* |
| How it is built | `NSURLSession` for HTTP, `flate2` + `tar` to unpack, `sha2` to verify; no subprocess other than our own binary |
| Relaunch | A hidden `vitals relaunch` helper that waits for the old tray to exit, then launches the new bundle |

## 1. Release pipeline

### `scripts/release.sh <x.y.z>`

The one way a version is cut. It:

1. Refuses to run unless the working tree is clean and the branch is `main`.
2. Sets `[workspace.package] version` in `Cargo.toml` and refreshes
   `Cargo.lock` (`cargo update -w --offline`).
3. Sets `CFBundleShortVersionString` to `x.y.z` and increments
   `CFBundleVersion` by one in `resources/Info.plist`.
4. Turns `## [Unreleased]` in `CHANGELOG.md` into `## [x.y.z] - <today>`,
   inserts a fresh empty `## [Unreleased]` above it, and updates the link
   references at the bottom of the file.
5. Commits `release: x.y.z` and creates the annotated tag `vx.y.z`.
6. Prints `git push --follow-tags` and stops. The push stays manual.

`--dry-run` skips step 1 and the `Cargo.lock` refresh, applies the three
text edits of steps 2–4 to temporary copies of the files, and prints the
diff without committing or tagging. CI runs the dry run on every push, so
a broken script fails before anyone tags.

Versions are plain `MAJOR.MINOR.PATCH`. A version with a hyphen suffix
(`0.2.0-beta.1`) is accepted and produces a GitHub pre-release, which
`releases/latest` never returns, so installed apps never see it.

### `.github/workflows/release.yml`

Triggered by a `v*` tag push. Runs on `macos-15` (arm64), the same runner
as `ci.yml`. Steps:

1. **Version check**: the tag (minus `v`), `Cargo.toml`'s workspace
   version and `Info.plist`'s `CFBundleShortVersionString` must all be
   equal, or the job fails before building anything.
2. **Build**: dashboard `npm ci && npm run build`, then `make bundle` —
   the same commands as CI.
3. **Pack**: `tar -C dist -czf Vitals-x.y.z-arm64.tar.gz Vitals.app`.
   `Vitals.app/` is the only top-level entry.
4. **Sums**: `shasum -a 256 Vitals-x.y.z-arm64.tar.gz > SHA256SUMS`
   (two-space format: `<hex>  <filename>`).
5. **Notes**: `scripts/changelog-section.sh x.y.z` prints the body of
   that version's section from `CHANGELOG.md`.
6. **Publish**: `gh release create vx.y.z --title "Vitals x.y.z"
   --notes-file <notes> Vitals-x.y.z-arm64.tar.gz SHA256SUMS`, with
   `--prerelease` when the version has a hyphen suffix. Uses the
   workflow's `GITHUB_TOKEN` with `contents: write`.

### The contract the app relies on

- Asset names: `Vitals-<version>-arm64.tar.gz` and `SHA256SUMS`.
- Tarball layout: exactly one top-level entry, `Vitals.app/`.
- The bundle's `Info.plist` version equals the release version.
- Asset download URLs are
  `https://github.com/billsun9305/vitals/releases/download/v<version>/<name>`.

README's "Releasing a new version" section is rewritten to describe the
script and this contract.

## 2. The checker

### Module layout

New module `crates/app/src/update/` in the app crate:

| File | Job | AppKit? |
|---|---|---|
| `release.rs` | `Version`, `Release`, `Source`, GitHub JSON parsing, `SHA256SUMS` parsing, URL validation. Pure. | no |
| `http.rs` | `fetch(url) -> Result<Vec<u8>>` and `download(url, to_dir) -> Result<PathBuf>` over `NSURLSession`; results come back through a channel from the completion block. | Foundation only |
| `install.rs` | Stage, download, verify, unpack, check, spawn helper, swap. | Foundation only |
| `checker.rs` | Timers, state, the main-thread hand-off, menu wiring hooks. | yes |
| `relaunch.rs` | The `vitals relaunch` helper's body. | Foundation + `NSWorkspace` |

### Types (in `release.rs`)

```rust
pub struct Version { major: u32, minor: u32, patch: u32 }   // Ord; parse("v0.2.0") ok; parse("0.2.0-beta.1") -> Err
pub struct Source { pub api_latest: String, pub download_base: String }
impl Source {
    /// api_latest = "https://api.github.com/repos/billsun9305/vitals/releases/latest"
    /// download_base = "https://github.com/billsun9305/vitals/releases/download/"
    pub fn github() -> Source;
    /// Accepts only `http://127.0.0.1:<port>/` and `http://localhost:<port>/`; both
    /// fields point at that base. Anything else is `Err(UpdateError::BadSource)`.
    pub fn loopback(url: &str) -> Result<Source, UpdateError>;
}
pub struct Release { pub version: Version, pub notes: String, pub page_url: String, pub archive_url: String, pub sums_url: String, pub archive_name: String }
pub fn parse_latest(json: &[u8], source: &Source) -> Result<Release, UpdateError>
pub fn parse_sums(text: &str) -> Vec<(String, String)>   // (hex digest, file name)
pub fn expected_sum(sums: &str, name: &str) -> Option<[u8; 32]>
```

`parse_latest` rejects drafts and pre-releases, requires both assets by
name, and requires each asset's `browser_download_url` to start with
`source.download_base` and end with the asset name. Anything else is
`UpdateError::BadRelease(reason)`.

### Schedule

- One check 30 s after the tray starts (a one-shot `NSTimer`).
- Then one repeating `NSTimer` every 24 h with `tolerance = 3600 s`, so
  macOS can coalesce it with other wake-ups.
- *Check for Updates…* in the dropdown runs one immediately.

Only when the tray runs from a bundle: if
`NSBundle::mainBundle().bundleURL()` does not end in `.app`, no timer is
created and no update items are added to the menu.

### The request

`GET <source.api_latest>` with headers `Accept:
application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28`,
`User-Agent: vitals/<CARGO_PKG_VERSION>`. Timeout 10 s. Nothing else is
sent. The unauthenticated limit is 60 requests per hour per IP; a daily
check is nowhere near it.

The fetch runs on a background thread. Its result is boxed and handed to
the controller with `performSelectorOnMainThread:` — the pattern
`powerChanged:` already uses — so the menu never blocks.

### State and decision

On the controller:

```rust
struct UpdateState { available: Option<Release>, checking: bool, installing: bool, last_check: Option<(Instant, Result<(), String>)> }
```

`available` is `Some` when `release.version > Version::parse(CARGO_PKG_VERSION)`.
It is replaced by every successful check (a newer "latest" wins; an
older or equal one clears it). `Later` does not touch it.

### Failure

An automatic check that fails (offline, 403 from rate limiting, 5xx,
malformed JSON, no assets) writes one line to stderr and waits for the
next timer. A manual check reports either way in an alert: *You're up to
date — Vitals 0.1.0 is the latest.* or *Couldn't check for updates:
<reason>.* A manual check that finds an update opens the install alert
directly.

## 3. The indicators

### Menu bar item

`format_title` is unchanged. In the controller's title update: if
`available.is_some()`, build an `NSAttributedString` from the title plus
`status_item::UPDATE_BADGE` (`" ●"`), with the badge range coloured
`NSColor::controlAccentColor()` and the same font as the digits, and call
`setAttributedTitle:`. Otherwise `setTitle:` exactly as today. The
comparison that avoids redundant `setTitle:` calls compares the
`(title, badge)` pair.

### Dropdown

Menu order, top to bottom:

1. `● Update to Vitals 0.2.0…` — only while `available.is_some()`. Attributed
   title with the same accent dot. Action `installUpdate:`. Disabled and
   retitled `Installing…` while an install runs.
2. separator (only with row 1)
3. the panel item (unchanged)
4. separator
5. Open Dashboard ⌘D (unchanged)
6. `Check for Updates…` — action `checkForUpdates:`. Disabled and retitled
   `Checking…` while a check runs. Present whenever the feature is on.
7. Quit vitals (unchanged)

Items are inserted or removed in `menuWillOpen:` from the current state,
so nothing is redrawn while the menu is closed.

## 4. The installer

### The alert

`NSAlert` with `NSAlertStyle::Informational`, title `Vitals <new> is available`,
informative text = release notes as plain text, truncated at 1,500
characters with `…`. Buttons, in order: `Install and Relaunch` (default),
`Later`, `View Release`. *View Release* opens `page_url` with
`NSWorkspace::openURL`. *Later* closes the alert and changes nothing.

### Install

Three functions in `install.rs`, sequenced by the controller on a
background thread; the first error stops the run:

```rust
pub fn prepare(source: &Source, release: &Release, app_url: &Path) -> Result<Staged, UpdateError>; // steps 1–4
pub struct Staged { dir: PathBuf, app: PathBuf }   // Drop removes `dir` if `swap_into` never ran
impl Helper { pub fn spawn(parent: u32, app_url: &Path) -> Result<Helper, UpdateError>; pub fn cancel(self); } // step 5; cancel = SIGTERM
impl Staged { pub fn swap_into(self, app_url: &Path) -> Result<(), UpdateError>; }                      // step 6
```

The steps:

1. **Stage.** `NSFileManager::URLForDirectory(NSItemReplacementDirectory,
   NSUserDomainMask, appropriateForURL: app_url, create: true)`. Same
   volume as the bundle, so the swap in step 6 is a rename.
2. **Download.** `SHA256SUMS`, then the tarball, into the staging directory with `NSURLSession` download tasks
   (`timeoutIntervalForRequest` 10 s, no cap on a transfer in progress).
3. **Verify.** SHA-256 (`sha2`) of the tarball must equal
   `expected_sum(sums, archive_name)`; otherwise `UpdateError::BadChecksum`.
4. **Unpack.** `flate2::read::GzDecoder` + `tar::Archive::unpack` into
   staging. Require the archive's top-level entries to be exactly
   `Vitals.app`. Then read `Vitals.app/Contents/Info.plist` with
   `NSDictionary::dictionaryWithContentsOfURL` and require
   `CFBundleShortVersionString == release.version`. Executable bits come
   from the tar entries. No quarantine attribute is set because no
   quarantine-aware app touched the bytes; the integration test asserts
   the attribute is absent.
5. **Spawn the helper.** `std::process::Command::new(current_exe())
   .args(["relaunch", "--parent", <pid>, "--app", <app path>])`, stdio
   null. The helper waits on the parent with `window::parent::wait_for_exit`
   (kqueue, already tested), then launches `--app` through
   `NSWorkspace::openApplicationAtURL_configuration_completionHandler`
   with `createsNewApplicationInstance = true`, then exits. If the helper
   fails to spawn, the install stops here.
6. **Swap.** `NSFileManager::replaceItemAtURL(app_url, withItemAtURL:
   staged_app, backupItemName: nil, options: [])`. The old bundle is
   discarded; the running process keeps its mapped binary.
7. **Terminate.** Back on the main thread, `NSApp.terminate`. The
   LaunchAgent has `KeepAlive=false`, so launchd does not relaunch the
   old copy; the helper launches the new one. The old status item is gone
   before the new one appears.

### Failure

Any error: delete the staging directory, `SIGTERM` the helper if it was
spawned, leave the installed bundle untouched, show one alert
`Couldn't install Vitals <new>` with the reason, and return the top row to
`● Update to Vitals <new>…`. Nothing retries on its own.

### Security

- The API host is fixed (`Source::github()`); the response's URLs are only
  accepted when they start with our own `releases/download/` prefix.
- The tarball's hash must match `SHA256SUMS`, which comes from the same
  release. This protects against corruption and mismatched assets, not
  against a compromised GitHub account; with ad-hoc signing there is
  nothing stronger to check, and the README's security section says so.
- `tar::Archive` is configured with `set_overwrite(false)` and unpacks into
  the fresh staging directory only; entries escaping the directory are
  rejected by the crate.
- The hidden `--update-source <url>` flag on the tray is accepted only for
  `http://127.0.0.1` / `http://localhost` URLs and exists for the manual
  end-to-end test.

### Budget

- Binary: `flate2`, `tar`, `sha2` are added to the app crate. The new
  size is measured and recorded in `docs/budget.md` against the 6 MB cap.
- Idle: one `NSTimer` per day with an hour of tolerance. No thread or
  socket exists between checks; each check's `NSURLSession` is created
  per check and `finishTasksAndInvalidate`d after it.
- Memory: tray footprint measured before and after a check, the same way
  the existing numbers were, and recorded in `docs/budget.md`.

### CLI additions

- Hidden `Command::Relaunch { parent: u32, app: PathBuf }`.
- Hidden `--update-source <url>` on the bare (tray) invocation.

## 5. Testing

### Unit (pure)

In `release.rs`: `Version` parse/compare (`v` prefix ok, `-beta` rejected,
`0.10.0 > 0.9.9`); a fixture of GitHub's `releases/latest` JSON parsed to
a `Release`; drafts and pre-releases rejected; missing asset rejected;
asset URL on a different host or path rejected; `SHA256SUMS` parsing and
lookup by name. In `status_item.rs`: the `(title, badge)` change
detection. `Source::loopback` accepts `http://127.0.0.1:<port>/` and
`http://localhost:<port>/` and rejects anything else.

### Integration — `crates/app/tests/update.rs`

A `tiny_http` fixture (already a dependency; same pattern as
`tests/serve.rs`) serving `/releases/latest`, `/SHA256SUMS` and the
tarball, where the tarball is built by the test from a temporary bundle
whose `Info.plist` carries the target version. The test uses
`Source::loopback(fixture_url)` and a temporary "installed" bundle. Cases:

1. Happy path: the installed bundle is replaced; its `Info.plist` now
   carries the new version; the staging directory is gone; the new
   bundle has no `com.apple.quarantine` attribute. (The test calls `prepare` then
   `swap_into` and never `Helper::spawn`, so no process is launched from
   a test.)
2. Wrong hash: `Err(BadChecksum)`, installed bundle byte-for-byte
   unchanged, staging gone.
3. Version mismatch inside the tarball: `Err(BadRelease)`, unchanged.
4. Two top-level entries in the tarball: `Err(BadRelease)`.
5. Server returns 404 / 403 / connection refused: `Err(Http(..))` with a
   reason string, unchanged.

Loopback HTTP through `NSURLSession` is already known to work in this
project: the dashboard window loads `http://127.0.0.1:9876/` the same way.

### Scripts

CI runs `scripts/release.sh --dry-run 9.9.9` and
`scripts/changelog-section.sh Unreleased` and fails if either errors or
prints nothing.

### Manual, before the first tag

Build a `0.1.0` and a `0.1.1` bundle locally; serve the `0.1.1` tarball,
`SHA256SUMS` and a hand-written `latest.json` with any static file server;
install `0.1.0` to `/Applications` and run it with `--update-source`;
confirm the dot, the row, the alert, the relaunch, the new version in the
menu, and the tray footprint. Record the result in `docs/budget.md`.

## Documentation changes

- README: rewrite "Releasing a new version"; add an "Updates" paragraph
  to the menu bar section (what is checked, how often, what is sent).
- CONTRIBUTING: the `--update-source` flag and the manual test.
- SECURITY: what the hash does and does not protect against.
- CHANGELOG `[Unreleased]`: the feature.
- `docs/budget.md`: the binary size and the footprint numbers.

## Order of work

The updater must be in the first tagged release, or the copies installed
from it can never learn about the second. So: implement, measure, then
`scripts/release.sh 0.1.0`.
