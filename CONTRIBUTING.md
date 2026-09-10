# Contributing to vitals

Thanks for looking. vitals is small enough that one person can hold the
whole thing in their head, and the rules below exist to keep it that way.

## What you need

- An Apple Silicon Mac on macOS 14 or newer. There is no Intel build and no
  Linux build: the sampler reads Apple's IOReport, which only exists here.
- Rust, via `rustup`. `rust-toolchain.toml` selects the current stable
  toolchain with the `aarch64-apple-darwin` target, and rustup installs it
  the first time you run `cargo`. The minimum supported version is the
  `rust-version` in `Cargo.toml`.
- Node 22 or newer, for the dashboard in `dashboard/`.

No `sudo`, no entitlements, no signing identity — the app bundle is ad-hoc
signed and only ever installed on your own machine.

## The loop

```bash
make build   # cargo build --release
make test    # cargo test --workspace
make lint    # cargo clippy --workspace --all-targets --examples -- -D warnings
make fmt     # cargo fmt --all
```

`make lint` must be clean, not merely free of hard errors: CI runs clippy
with `-D warnings`.

CI runs on GitHub's `macos-15` runners, which are virtual machines: Apple
Silicon underneath, but with no IOReport CPU channels, so the sampler cannot
be built there. Tests that need a real sample begin with
`vitals_core::skip_without_hardware!()` and print `skipped:` on the runner
instead of failing. So CI proves the pure half of the suite and the build;
the sampling half only runs on a real Mac. Run `make test` locally before
merging anything that touches the sampler, the CLI verbs or the server.

The dashboard is embedded into the binary at build time:

```bash
cd dashboard
npm ci
npm run lint     # oxlint
npm run build    # writes dashboard/dist, which `cargo build` picks up
```

so after changing anything under `dashboard/src`, run `npm run build`
*and then* `cargo build` — the binary does not notice the page changed on
its own. For iterating on the page, `npm run dev` serves the source with
hot reload and proxies `/api/*` to whatever `vitals serve` is on port 9876
(the menu bar app's own server counts).

To look at the menu bar dropdown without opening a menu — both themes, the
empty state, the error state — `cargo run --release --example render_panel
-- /tmp/panel` paints the same `drawRect:` to PNG files.

## Where things live

| Path | What it is |
|---|---|
| `crates/core` | Sampling, the JSON schema, the pressure verdict, process ranking. Pure Rust, no AppKit, heavily unit-tested. |
| `crates/app` | The `vitals` binary: CLI verbs, the localhost server, the menu bar tray and its window. |
| `dashboard/` | The Vite + React page the server embeds. |
| `docs/agents.md` | The JSON contract as an agent reads it. Update it whenever output changes. |
| `docs/budget.md` | Every performance number this project has promised, with how it was measured. |
| `resources/` | `Info.plist` for the bundle and the LaunchAgent plist. |
| `scripts/` | The bundle script `make bundle` runs. |

## The rules that are not style

1. **The budget is a contract.** Idle CPU under 0.3%, memory under 25 MB,
   binary under 6 MB — see `docs/budget.md` for the current numbers and the
   method. A change to `crates/app/src/tray`, `crates/app/src/serve` or
   the sampler cadence must come with a measurement in the PR, taken the
   same way the doc describes, and the doc updated if a number moved.
2. **Nothing runs while nobody is looking.** The dropdown redraws only
   while open; the dashboard polls only while visible; the server starts on
   demand. A feature that needs a timer, a socket or a thread to exist
   while idle needs a very good reason.
3. **No `sudo`, no subprocesses, no `powermetrics`.** Everything is read
   in-process.
4. **Pure decisions get unit tests; AppKit does not get unit tests.**
   Extract the arithmetic or the rule into a function and test that (see
   `tray/panel.rs` and `tray/status_item.rs` for the pattern). Verify the
   AppKit side by running it — the `render_panel` example exists for
   exactly this.
5. **The JSON output is versioned.** If a field is added, removed or
   changes meaning, bump `schema_version` in `crates/core/src/schema.rs`,
   mirror the change in `dashboard/src/types.ts`, and update
   `docs/agents.md`.
6. **Every `unsafe` block carries a `// SAFETY:` comment** naming the
   contract it upholds. objc2 makes most of AppKit safe; where it can't,
   say why the call is sound.

## Commits and pull requests

- Commit messages follow the existing history: `feat(tray): …`,
  `fix(serve): …`, `docs: …`, `refactor(core): …`. The body says *why*, and
  names the measurement when there is one.
- Keep a pull request to one change. Split a refactor from the behaviour
  change it enables.
- CI (`.github/workflows/ci.yml`) runs fmt, clippy, the test suite, the
  dashboard lint and build, and the bundle script on an Apple Silicon
  runner. It has to be green.
- If you are changing what the tray or the dashboard looks like, put a
  screenshot (or the `render_panel` PNGs) in the PR.

## Reporting bugs

Use the bug report template. The most useful thing you can attach is the
output of `vitals snapshot` — it carries the chip, the core layout and the
macOS-visible state the bug happened under.

## Security

See [SECURITY.md](SECURITY.md). Please don't open a public issue for a
vulnerability.
