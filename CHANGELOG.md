# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

The first release. Everything below is new.

### Added

- **CLI** — `vitals snapshot`, `top`, `pressure` and `watch`: CPU, GPU,
  memory, swap, power, temperature, thermal state and per-core detail for
  Apple Silicon, sampled in-process from IOReport with no `sudo` and no
  subprocess, as versioned JSON (`schema_version`) with `--human` variants.
- **`vitals pressure`** — a semantic verdict (`nominal` / `warning` /
  `critical`) with reasons, the likely process, a one-sentence summary and
  an optional exit code, built to be quoted by an agent rather than parsed.
- **Menu bar app** — the same binary with no arguments is an `NSStatusItem`
  with live text and a custom-drawn dropdown panel (CPU with per-core bars,
  GPU, memory judged by the kernel's pressure signal, power). Nothing
  redraws while the dropdown is closed; sampling backs off while the
  display sleeps. `make install-app` installs it to `/Applications`.
- **Localhost server and dashboard** — `vitals serve` exposes
  `/api/snapshot`, `/api/top`, `/api/pressure` and `/api/version` on
  `127.0.0.1` only,
  with a strict `Host` check, and serves an embedded React dashboard:
  verdict card, stat tiles with trends, a 2/5/15-minute range, CPU and GPU
  charts with crosshair and tooltip, per-core bars, memory and swap meters,
  a ranked process table, a table view for every chart, and light/dark
  themes. Polling stops whenever the page is hidden.
- **Native dashboard window** — *Open Dashboard* in the tray (or a Finder
  double-click on the running app) opens the dashboard in a `WKWebView`
  window with a Dock tile and app menu that exist only while it is open.
- **`docs/agents.md`** — the JSON contract written for agents, and
  **`docs/budget.md`** — every performance promise with its measurement.
- **Releases** — `scripts/release.sh` cuts a version; the tag builds,
  signs, notarizes and publishes `Vitals-<version>.dmg`, the updater's
  tarball and `SHA256SUMS` on GitHub Releases.
- **In-app updates** — the menu bar app checks GitHub Releases daily and
  on demand, shows an accent dot and an *Update to Vitals x.y.z…* row, and
  installs with one click after verifying the hash and, for a signed copy,
  the Developer ID signature.
- **Start at Login** — the app registers itself with `SMAppService` on its
  first launch, with a toggle in the menu. There is no LaunchAgent.

[Unreleased]: https://github.com/billsun9305/vitals/commits/main
