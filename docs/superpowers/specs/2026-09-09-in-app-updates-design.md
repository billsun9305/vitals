# In-app updates and distribution — design

Date: 2026-09-09. Status: sections 1–6 approved in conversation, awaiting written review.

## Goal

A new user installs vitals by downloading a notarized disk image from the
GitHub Releases page, with no GitHub account and no Apple account, drags it
to Applications, and it starts at login from then on. When a new version
is published, every installed menu bar copy shows that an update exists
and can install it in one click, without the user visiting GitHub,
downloading anything by hand, or re-running `make install-app`.

## Non-goals

- No automatic installs. The user always clicks *Install and Relaunch*.
- No "skip this version", no beta channel, no update settings UI.
- No Homebrew tap, no Intel build, no Mac App Store.
- Nothing for the CLI verbs. Updates and autostart are tray features.

## Decisions already made

| Question | Answer |
|---|---|
| Where releases come from | GitHub Releases for `billsun9305/vitals`, built by CI from a `v*` tag |
| How the user learns of one | An accent-coloured `●` after the digits in the menu bar, and a row at the top of the dropdown |
| What a click does | A confirmation alert with the release notes: *Install and Relaunch* / *Later* / *View Release* |
| How it is built | `NSURLSession` for HTTP, `flate2` + `tar` to unpack, `sha2` to verify; no subprocess other than our own binary |
| Relaunch | A hidden `vitals relaunch` helper that waits for the old tray to exit, then launches the new bundle |
| How a new user installs | `Vitals-x.y.z.dmg` from the Releases page, which is public: no GitHub account, no Apple account. Actions artifacts are not a download: they need a login, expire, and lose file permissions |
| Signing | Developer ID Application + hardened runtime + notarization + stapling, done by CI when the repository secrets exist; ad-hoc otherwise, and the release notes say which |
| Autostart | The app registers itself as a login item with `SMAppService.mainApp` on first bundled launch, with a *Start at Login* toggle in the dropdown. The LaunchAgent plist is removed |

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
2. **Build**: dashboard `npm ci && npm run build`, then `make bundle`,
   the same commands as CI. `make bundle` runs `scripts/bundle.sh`, which
   does the signing described next.
3. **Sign**: `scripts/bundle.sh` packs `dist/Vitals.app` and signs it with
   `$SIGN_IDENTITY`, default `-` (ad-hoc, what it does today). When the
   secret `MACOS_CERT_P12` exists, the workflow first imports the
   certificate into a temporary keychain (`security create-keychain`,
   `security import`, `security set-key-partition-list`) and sets
   `SIGN_IDENTITY` to the `Developer ID Application` identity. With a real
   identity `bundle.sh` adds `--options runtime --timestamp`; there is no
   entitlements file, because the app needs none.
4. **Notarize** (only when signed): `ditto -c -k --keepParent
   dist/Vitals.app Vitals.zip`, then `xcrun notarytool submit Vitals.zip
   --key <p8> --key-id $NOTARY_KEY_ID --issuer $NOTARY_ISSUER_ID --wait`,
   then `xcrun stapler staple dist/Vitals.app`. The ticket lives inside
   the bundle, so every later artefact carries it.
5. **Pack**: `tar -C dist -czf Vitals-x.y.z-arm64.tar.gz Vitals.app`.
   `Vitals.app/` is the only top-level entry. This is the updater's asset.
6. **Disk image**: a staging folder holding `Vitals.app` and an
   `Applications` symlink, then `hdiutil create -volname "Vitals x.y.z"
   -srcfolder <staging> -format UDZO Vitals-x.y.z.dmg`. When signed, the
   DMG is itself signed, notarized and stapled the same way. This is the
   human's download.
7. **Sums**: `shasum -a 256 Vitals-x.y.z-arm64.tar.gz Vitals-x.y.z.dmg >
   SHA256SUMS` (two-space format: `<hex>  <filename>`).
8. **Notes**: `scripts/changelog-section.sh x.y.z` prints the body of
   that version's section from `CHANGELOG.md`. When the build was not
   signed, the workflow appends a paragraph saying the app is ad-hoc
   signed and how to open it (System Settings, Privacy & Security, *Open
   Anyway*).
9. **Publish**: `gh release create vx.y.z --title "Vitals x.y.z"
   --notes-file <notes> Vitals-x.y.z.dmg Vitals-x.y.z-arm64.tar.gz
   SHA256SUMS`, with `--prerelease` when the version has a hyphen suffix.
   Uses the workflow's `GITHUB_TOKEN` with `contents: write`.

### Secrets

Five repository secrets, all created and set by the maintainer, never by
tooling: `MACOS_CERT_P12` (the Developer ID Application certificate
exported from Keychain Access, base64), `MACOS_CERT_PASSWORD`,
`NOTARY_KEY_ID`, `NOTARY_ISSUER_ID` and `NOTARY_KEY_P8` (an App Store
Connect API key with the Developer role). Step 3's import and step 4 run only when `MACOS_CERT_P12` is set, so a fork without secrets still produces a
working, ad-hoc release.

### The contract the app relies on

- Asset names: `Vitals-<version>-arm64.tar.gz`, `Vitals-<version>.dmg`
  and `SHA256SUMS`. The updater reads only the first and the last.
- Tarball layout: exactly one top-level entry, `Vitals.app/`.
- The bundle's `Info.plist` version equals the release version.
- A signed release's bundle carries a Developer ID signature with the
  maintainer's Team ID and a stapled notarization ticket.
- Asset download URLs are
  `https://github.com/billsun9305/vitals/releases/download/v<version>/<name>`.

README's "Releasing a new version" section is rewritten to describe the
script, the secrets and this contract. `scripts/bundle.sh` loses its
comment that the binary "will never leave this machine".

## 2. The checker

### Module layout

New module `crates/app/src/update/` in the app crate:

| File | Job | AppKit? |
|---|---|---|
| `release.rs` | `Version`, `Release`, `Source`, GitHub JSON parsing, `SHA256SUMS` parsing, URL validation. Pure. | no |
| `http.rs` | `fetch(url) -> Result<Vec<u8>>` and `download(url, to_dir) -> Result<PathBuf>` over `NSURLSession`; results come back through a channel from the completion block. | Foundation only |
| `install.rs` | Stage, download, verify, unpack, check version and signature, spawn helper, swap. | Foundation + Security |
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
   **Then check the signature.** Read the running bundle's Team ID with
   the Security framework (`SecCodeCopySelf`,
   `SecCodeCopySigningInformation`, `kSecCodeInfoTeamIdentifier`, through
   the `objc2-security` crate). If there is one, the staged bundle must
   pass `SecStaticCodeCheckValidity` against the requirement
   `anchor apple generic and certificate leaf[subject.OU] = "<team id>"`,
   or the install stops with `UpdateError::BadSignature`. An ad-hoc copy
   (a local build) has no Team ID and skips this check.
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
7. **Terminate.** Back on the main thread, `NSApp.terminate`. Nothing
   else relaunches the old copy: autostart is a login item (section 6),
   which only acts at login. The helper launches the new one, and the old
   status item is gone before the new one appears. The login item follows
   the bundle path, so it survives the swap.

### Failure

Any error: delete the staging directory, `SIGTERM` the helper if it was
spawned, leave the installed bundle untouched, show one alert
`Couldn't install Vitals <new>` with the reason, and return the top row to
`● Update to Vitals <new>…`. Nothing retries on its own.

### Security

- The API host is fixed (`Source::github()`); the response's URLs are only
  accepted when they start with our own `releases/download/` prefix.
- The tarball's hash must match `SHA256SUMS` from the same release; that
  catches corruption and mismatched assets. Authenticity is the signature
  check in step 4: when the running copy is Developer-ID-signed, the new
  bundle must be validly signed with the same Team ID, which a compromised
  GitHub account cannot produce. An ad-hoc copy has no Team ID and gets
  only the hash, and SECURITY.md says so.
- `tar::Archive` is configured with `set_overwrite(false)` and unpacks into
  the fresh staging directory only; entries escaping the directory are
  rejected by the crate.
- The hidden `--update-source <url>` flag on the tray is accepted only for
  `http://127.0.0.1` / `http://localhost` URLs and exists for the manual
  end-to-end test.

### Budget

- Binary: `flate2`, `tar`, `sha2`, `objc2-security` and
  `objc2-service-management` are added to the app crate. The new size is
  measured and recorded in `docs/budget.md` against the 6 MB cap.
- Idle: one `NSTimer` per day with an hour of tolerance. No thread or
  socket exists between checks; each check's `NSURLSession` is created
  per check and `finishTasksAndInvalidate`d after it.
- Memory: tray footprint measured before and after a check, the same way
  the existing numbers were, and recorded in `docs/budget.md`.

### CLI additions

- Hidden `Command::Relaunch { parent: u32, app: PathBuf }`.
- Hidden `Command::LoginItem { action: On | Off | Status }` (section 6).
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
   bundle has no `com.apple.quarantine` attribute; the signature check
   was skipped because the test binary has no Team ID, and the log says
   so. (The test calls `prepare` then
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
prints nothing. `make bundle` in CI runs `bundle.sh` with the default
ad-hoc identity, as today.

### Login item (section 6)

`login_item::menu_state` is pure and unit-tested for every
`SMAppServiceStatus` value. Registration itself is AppKit-side and is
verified by running it: the manual test below.

### Manual, before the first tag

1. **Hardened runtime.** Sign a local bundle with the Developer ID
   identity on this Mac (`SIGN_IDENTITY="Developer ID Application: …"
   make bundle`) and run the tray, the dashboard window and every CLI
   verb from it. Nothing in the app loads third-party code, so nothing
   should change, but this is the first time it runs hardened.
2. **Update.** Build a `0.1.0` and a `0.1.1` bundle, both signed the same
   way; serve the `0.1.1` tarball, `SHA256SUMS` and a hand-written
   `latest.json` with any static file server; install `0.1.0` to
   `/Applications` and run it with `--update-source`; confirm the dot, the
   row, the alert, the signature check passing, the relaunch, the new
   version in the menu, and the tray footprint. Then repeat once with a
   `0.1.1` signed ad-hoc and confirm `BadSignature` and an untouched
   `0.1.0`.
3. **Login item.** After `make install-app`, confirm Vitals appears under
   System Settings, General, Login Items, that the dropdown's *Start at
   Login* is checked, that unchecking it removes the entry, and that a
   log-out and log-in starts the tray.
4. **Download path.** After the first tagged release, download the DMG on
   this Mac with a browser, drag, open: no Gatekeeper dialog.

Record the numbers in `docs/budget.md`.

## 6. Install and autostart

### The app registers itself

New module `crates/app/src/tray/login_item.rs`, over
`objc2-service-management`'s `SMAppService` (`mainAppService`,
`registerAndReturnError`, `unregisterAndReturnError`, `status`,
`openSystemSettingsLoginItems`; macOS 13+, our floor is 14):

```rust
pub enum LoginStatus { Enabled, NotRegistered, RequiresApproval, NotFound }
pub fn status() -> LoginStatus;
pub fn set(on: bool) -> Result<(), String>;
/// First bundled launch only: registers, then records `registeredAtLogin = true`
/// in NSUserDefaults so a user who later turns it off is not re-enrolled.
pub fn register_once();
/// Pure: (title, checked, enabled) for the menu item.
pub fn menu_state(status: LoginStatus) -> (&'static str, bool, bool);
```

`register_once` runs when the tray starts from a bundle, after the status
item exists. `menu_state` maps `Enabled` to (`Start at Login`, checked,
enabled), `NotRegistered` to (`Start at Login`, unchecked, enabled),
`RequiresApproval` to (`Start at Login — approve in System Settings…`,
unchecked, enabled; clicking opens the Login Items pane), and `NotFound`
to (`Start at Login`, unchecked, disabled).

### Dropdown

`Start at Login` sits between *Open Dashboard* and *Check for Updates…*,
refreshed from `status()` in `menuWillOpen:` like the update rows. Only
when running from a bundle.

### The LaunchAgent goes away

`resources/com.billsun.vitals.plist` is deleted. `make install-app`
becomes: `make bundle`, copy to `/Applications`, symlink `$(PREFIX)/bin/vitals`,
`open /Applications/Vitals.app`. It also removes a
`~/Library/LaunchAgents/com.billsun.vitals.plist` left by an earlier
install, after `launchctl unload`, so nothing launches twice.
`make uninstall-app` runs `vitals login-item off` from the installed
bundle, quits the tray, and removes the bundle and the symlink. The hidden
`Command::LoginItem` exists for these two targets and the manual test.

### README "Install", rewritten

Three subsections, in this order:

1. **Download** — Requirements (Apple Silicon, macOS 14 or newer). Open
   the Releases page, download `Vitals-x.y.z.dmg`, drag to Applications,
   open. It is notarized, so it opens without a dialog, and it starts at
   login from now on, with the toggle in the menu. If a release's notes
   say the build is unsigned, the way to open it is System Settings,
   Privacy & Security, *Open Anyway*.
2. **From source, menu bar app** — `git clone`, Rust via rustup, Node 22,
   then `make install-app PREFIX=$HOME/.local`. No `sudo` anywhere; the
   current advice to run the target under `sudo` is removed, because it
   would run `npm ci`, `cargo build` and the app launch as root.
3. **From source, CLI only** — `make install PREFIX=$HOME/.local`.

## Documentation changes

- README: the Install section above; rewrite "Releasing a new version";
  an "Updates" paragraph in the menu bar section (what is checked, how
  often, what is sent); the LaunchAgent paragraph replaced by the login
  item.
- CONTRIBUTING: `SIGN_IDENTITY`, the `--update-source` flag, the hidden
  verbs, and the manual tests.
- SECURITY: the signature check, and what the hash alone protects
  against on an ad-hoc copy.
- CHANGELOG `[Unreleased]`: both features.
- `docs/budget.md`: the binary size and the footprint numbers.

## Order of work

The updater and the login item must both be in the first tagged release,
or the copies installed from it can never learn about the second and do
not start at login. So: implement, run the manual tests, add the five
secrets, then `scripts/release.sh 0.1.0`.
