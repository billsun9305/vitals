# vitals

[![CI](https://github.com/billsun9305/vitals/actions/workflows/ci.yml/badge.svg)](https://github.com/billsun9305/vitals/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platform: Apple Silicon macOS 14+](https://img.shields.io/badge/platform-Apple%20Silicon%20%C2%B7%20macOS%2014%2B-lightgrey)

Apple Silicon system monitor. The CLI is a single Rust binary that samples
CPU, GPU, memory, power, and thermal state in-process — no `powermetrics`,
no `sudo`, no subprocess of any kind — and prints structured JSON an agent
(or a script, or you) can consume directly.

Today `vitals` is a CLI with four verbs: `snapshot`, `top`, `pressure`,
`watch` (plus `serve`/`dashboard`, a local web dashboard — see
`vitals --help`). The same binary, run with no arguments, is also a menu
bar tray item; see [below](#the-menu-bar-app) for what it does, and
[Install](#install) for the download.

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

## The four verbs

Every example below is real output from a run on this machine — the values
are not invented. The only editing is to the `cores` array, whose ten
entries are shown one-per-line here and elided as `[...]` further down;
`vitals` itself pretty-prints each core object across eight lines, which
would run to eighty lines in a README.

JSON is the default for `snapshot`, `top`,
and `pressure`; each also accepts `--human` for a short text form meant
for a terminal, not a parser. `snapshot`, `top` and `pressure` also accept
`--json` as a no-op, since JSON is already their default, so a script that
passes it explicitly does not break. `watch` takes neither flag: it is
always NDJSON, and `vitals watch --json` is an argument error (exit 2).

### `vitals snapshot` — one full SoC sample

```
$ vitals snapshot
{
  "schema_version": 1,
  "sampled_at": "2026-09-08T09:24:41Z",
  "sample_ms": 200,
  "host": {
    "chip": "Apple M1 Pro",
    "model": "MacBookPro18,3",
    "ecpu_cores": 2,
    "pcpu_cores": 8,
    "gpu_cores": 14,
    "ncpu": 10
  },
  "cpu_pct": 60.6,
  "cpu_scaled_pct": 60.6,
  "ecpu_pct": 100.0,
  "ecpu_freq_mhz": 2064,
  "pcpu_pct": 50.8,
  "pcpu_freq_mhz": 3224,
  "cores": [
    { "id": 0, "kind": "E", "die_id": 0, "core_id": 0, "pct": 100.0, "freq_mhz": 2064 },
    { "id": 1, "kind": "E", "die_id": 0, "core_id": 10, "pct": 100.0, "freq_mhz": 2064 },
    { "id": 2, "kind": "P", "die_id": 0, "core_id": 0, "pct": 79.3, "freq_mhz": 3228 },
    { "id": 3, "kind": "P", "die_id": 0, "core_id": 10, "pct": 69.6, "freq_mhz": 3228 },
    { "id": 4, "kind": "P", "die_id": 0, "core_id": 20, "pct": 79.2, "freq_mhz": 3228 },
    { "id": 5, "kind": "P", "die_id": 0, "core_id": 30, "pct": 62.2, "freq_mhz": 3228 },
    { "id": 6, "kind": "P", "die_id": 0, "core_id": 100, "pct": 74.6, "freq_mhz": 3221 },
    { "id": 7, "kind": "P", "die_id": 0, "core_id": 110, "pct": 30.1, "freq_mhz": 3225 },
    { "id": 8, "kind": "P", "die_id": 0, "core_id": 120, "pct": 3.8, "freq_mhz": 3214 },
    { "id": 9, "kind": "P", "die_id": 0, "core_id": 130, "pct": 7.5, "freq_mhz": 3221 }
  ],
  "gpu_pct": 0.0,
  "gpu_freq_mhz": 0,
  "mem_total_mb": 32768,
  "mem_used_mb": 27934,
  "swap_total_mb": 16384,
  "swap_used_mb": 15977,
  "mem_pressure": "warning",
  "power_total_w": 13.52,
  "power_cpu_w": 13.52,
  "power_gpu_w": 0.0,
  "power_ane_w": 0.0,
  "power_ram_w": 1.45,
  "power_sys_w": 42.54,
  "temp_cpu_c": 75.2,
  "temp_gpu_c": 9.2,
  "fans": [
    { "name": "fan0", "rpm": 2299, "max_rpm": 5779 },
    { "name": "fan1", "rpm": 2509, "max_rpm": 6241 }
  ],
  "load_avg": [29.97900390625, 32.974609375, 26.58984375],
  "uptime_s": 1238588,
  "thermal_state": "nominal"
}
```

`--human` prints one line instead:

```
$ vitals snapshot --human
cpu 82.5%  gpu 0.0%  mem 27337/32768 MB  swap 15977 MB  20.40 W  73.8°C
```

`--interval <ms>` changes the sampling window (default `200`); it is the
window `macmon` integrates CPU/GPU/power residency over, so a longer
window smooths a spiky workload at the cost of a slower response.

The `cores` array on this machine (an M1 Pro) has 2 E-cores then 8 P-cores,
concatenated with a dense `id`; `cpu_pct` is utilization (active-residency
fraction) and `cpu_scaled_pct` is that same residency weighted by clock
speed against each core's maximum — the two read the same on a fully
loaded machine like this one, and diverge by roughly 3x on an idle one.

### `vitals top -n 5` — who is using CPU and memory

```
$ vitals top -n 5
{
  "schema_version": 1,
  "sampled_at": "2026-09-08T09:24:47Z",
  "sample_ms": 200,
  "by_cpu": [
    { "pid": 20087, "name": "recovery-11ffe4f4123ffddd", "app": "ChatGPT", "cpu_pct": 100.3, "mem_mb": 37, "threads": 13 },
    { "pid": 21100, "name": "parpmux", "app": "ChatGPT", "cpu_pct": 100.0, "mem_mb": 13, "threads": 12 },
    { "pid": 21291, "name": "rustc", "app": "claude", "cpu_pct": 99.8, "mem_mb": 827, "threads": 4 },
    { "pid": 21286, "name": "mdworker_shared", "cpu_pct": 24.9, "mem_mb": 26, "threads": 5 },
    { "pid": 21126, "name": "mdworker_shared", "cpu_pct": 23.4, "mem_mb": 27, "threads": 5 }
  ],
  "by_mem": [
    { "pid": 63306, "name": "node", "app": "LM Studio", "cpu_pct": 0.0, "mem_mb": 6654, "threads": 24 },
    { "pid": 21765, "name": "Codex (Renderer)", "app": "ChatGPT", "cpu_pct": 0.0, "mem_mb": 1338, "threads": 21 },
    { "pid": 21291, "name": "rustc", "app": "claude", "cpu_pct": 99.8, "mem_mb": 827, "threads": 4 },
    { "pid": 93134, "name": "Google Chrome Helper (Renderer)", "app": "Google Chrome", "cpu_pct": 0.0, "mem_mb": 444, "threads": 22 },
    { "pid": 89232, "name": "Claude Helper (Renderer)", "app": "Claude", "cpu_pct": 0.0, "mem_mb": 310, "threads": 25 }
  ]
}
```

`sample_ms` here is `200` — the window `sysinfo` measures each process's
CPU delta over, fixed and not currently configurable per invocation.
`cpu_pct` is **per process**, not system-wide, and exceeds 100 on
multi-core work exactly like `ps` does (`rustc` above is using nearly a
full core). `-n` controls rows per ranking, independently for `by_cpu` and
`by_mem`, so the same process can legitimately appear in both.

`--human` prints two aligned tables instead:

```
$ vitals top -n 5 --human
BY CPU
    PID     CPU%    MEM MB  NAME
  21100     99.0        13  parpmux
  20087     98.9        38  recovery-11ffe4f4123ffddd
  21291     96.6      1643  rustc
  21317     24.3        24  mdworker_shared
  21329     16.2        19  mdworker_shared
BY MEM
    PID     CPU%    MEM MB  NAME
  63306      0.0      6654  node
  21291     96.6      1643  rustc
  21765      9.3      1442  Codex (Renderer)
  93134      0.2       704  Google Chrome Helper (Renderer)
  89232      0.0       301  Claude Helper (Renderer)
```

### `vitals pressure` — a verdict, not raw numbers

This machine genuinely was under memory pressure while these docs were
written, so this is real, unedited output — not a cherry-picked example:

```
$ vitals pressure
{
  "schema_version": 1,
  "sampled_at": "2026-09-08T09:24:52Z",
  "state": "warning",
  "history": true,
  "reasons": [
    { "code": "mem_pressure_warning", "severity": "warning", "detail": "kernel memory pressure level: warning" },
    { "code": "cpu_saturated", "severity": "warning", "detail": "1-minute load is 3.8x the core count" }
  ],
  "suspects": [
    { "pid": 63306, "name": "node", "app": "LM Studio", "why": "largest resident set (6654 MB)" },
    { "pid": 20087, "name": "recovery-11ffe4f4123ffddd", "app": "ChatGPT", "why": "highest CPU (102%)" }
  ],
  "summary": "Memory is under pressure and CPU load is 3.8x the core count. LM Studio is the likely cause, and ChatGPT is the busiest process."
}
```

`--human` prints just `summary`:

```
$ vitals pressure --human
Memory is under pressure and CPU load is 4.5x the core count. LM Studio is the likely cause, and ChatGPT is the busiest process.
```

`pressure` exits `0` by default, deliberately — reporting a bad state is
not the same as the command failing. Pass `--exit-code` to fold the
verdict severity into the exit status instead: `0` nominal, `3` warning,
`4` critical. A runtime failure — the sampler erroring, say — exits `1`
either way. A malformed invocation never reaches that path: the argument
parser rejects it first and exits `2`, so `1` and `2` distinguish "the
tool broke" from "you called it wrong". Verified on this machine:

```
$ vitals pressure --exit-code >/dev/null; echo $?
4
```

(`4` here, not `3` — CPU load had climbed to 4.8x the core count by the
time this ran a few seconds after the example above, crossing this
project's critical threshold.)

`history: false` appears instead of `true` when there is no usable prior
observation (first run, or the last one is stale) — the swap-trend reasons
are simply skipped, and every other rule still runs. Verified by deleting
`~/.cache/vitals/last.json` and re-running: `state` stayed `critical`,
only `history` flipped to `false`.

### `vitals watch -i 2` — a live stream, not a snapshot

The only verb that doesn't exit on its own. It prints one **compact**
(not pretty-printed) JSON object per line, flushing after every line, and
runs until interrupted or until `-n` samples have been emitted:

```
$ vitals watch -i 2 -n 2
{"schema_version":1,"sampled_at":"2026-09-08T09:25:08Z","sample_ms":2000,"host":{"chip":"Apple M1 Pro","model":"MacBookPro18,3","ecpu_cores":2,"pcpu_cores":8,"gpu_cores":14,"ncpu":10},"cpu_pct":42.7,"cpu_scaled_pct":42.5,"ecpu_pct":100.0,"ecpu_freq_mhz":2064,"pcpu_pct":28.3,"pcpu_freq_mhz":3094,"cores":[...],"gpu_pct":0.0,"gpu_freq_mhz":0,"mem_total_mb":32768,"mem_used_mb":27663,"swap_total_mb":16384,"swap_used_mb":15977,"mem_pressure":"warning","power_total_w":8.04,"power_cpu_w":8.04,"power_gpu_w":0.0,"power_ane_w":0.0,"power_ram_w":0.9,"power_sys_w":21.83,"temp_cpu_c":68.0,"temp_gpu_c":9.2,"fans":[{"name":"fan0","rpm":2306,"max_rpm":5779},{"name":"fan1","rpm":2483,"max_rpm":6241}],"load_avg":[50.78173828125,37.607421875,28.44287109375],"uptime_s":1238615,"thermal_state":"nominal"}
{"schema_version":1,"sampled_at":"2026-09-08T09:25:10Z","sample_ms":2000,"host":{"chip":"Apple M1 Pro","model":"MacBookPro18,3","ecpu_cores":2,"pcpu_cores":8,"gpu_cores":14,"ncpu":10},"cpu_pct":42.9,"cpu_scaled_pct":42.8,"ecpu_pct":100.0,"ecpu_freq_mhz":2064,"pcpu_pct":28.6,"pcpu_freq_mhz":3202,"cores":[...],"gpu_pct":0.0,"gpu_freq_mhz":0,"mem_total_mb":32768,"mem_used_mb":27663,"swap_total_mb":16384,"swap_used_mb":15977,"mem_pressure":"warning","power_total_w":8.2,"power_cpu_w":8.2,"power_gpu_w":0.0,"power_ane_w":0.0,"power_ram_w":0.93,"power_sys_w":17.91,"temp_cpu_c":67.3,"temp_gpu_c":9.2,"fans":[{"name":"fan0","rpm":2305,"max_rpm":5779},{"name":"fan1","rpm":2513,"max_rpm":6241}],"load_avg":[56.884765625,39.09130859375,29.02001953125],"uptime_s":1238617,"thermal_state":"nominal"}
```

(The `cores` array is elided above with `[...]` only to keep this README
readable — the real output has the full array on every line, same shape
as the `snapshot` example.) Each object is a full `snapshot` — note
`sample_ms` here is `2000`, matching `-i 2`: `watch`'s window is the whole
inter-sample interval, unlike the fixed 200ms window `snapshot` and `top`
use by default. `-i` sets both the interval and the sampling window; `-n`
stops after that many samples (`0`, the default, means run forever).

Because it's flushed per line, a consumer that reads one line and closes
the pipe gets a clean exit. Verified on this machine:

```
$ vitals watch | head -1 > /dev/null; echo $?
0
```

## Reading the output without misreading it

- Every size is MB, every power is watts, every temperature is Celsius —
  the unit lives in the key name (`mem_used_mb`, `power_total_w`,
  `temp_cpu_c`), never in a separate field.
- `schema_version` is on every response. `null` never appears — an absent
  optional field (like a fan's `max_rpm`, or a process's `app`) is simply
  omitted from the object, not present with a null value.
- `cpu_pct` means two different things depending on the verb: system-wide
  utilization in `snapshot`, per-process in `top`. Don't compare them
  directly.

See [`docs/agents.md`](docs/agents.md) for the condensed version of this
section aimed at an agent deciding which command to call.

## Why not Stats or iStat Menus?

Both are good, mature menu bar monitors for macOS, and neither is being
replaced here — the gap `vitals` fills is different. Stats and iStat Menus
are built for a human looking at a menu bar: they don't expose a CLI or a
structured API, so a script or an agent that wants "is anything wrong
right now" has no way to ask either of them directly — the only options
are screen-scraping a GUI or shelling out to `powermetrics` yourself
(which needs `sudo`). `vitals` is that missing entry point: one binary,
JSON on stdout, zero privilege escalation, meant to be called
programmatically rather than glanced at.

## The menu bar app

`vitals` run with no arguments is a menu bar tray: an `NSStatusItem`
showing live text plus a custom-drawn dropdown in four sections — CPU (a
sparkline of the time the menu has been open, then one bar per core with
efficiency and performance cores tinted differently), GPU, memory (judged by
the kernel's own pressure signal, not just the used fraction), and power.
Headline values turn orange at 70% and red at 90%. Nothing redraws while
the dropdown is closed, and the sampling cadence backs off automatically
while the display sleeps. See `docs/budget.md` for the measured idle cost.

To look at the dropdown without opening a menu — for instance to check it in
both themes, or its empty and error states — `cargo run --release --example
render_panel -- /tmp/panel` paints the same `drawRect:` to PNG.

**It has no window until you ask for one.** `LSUIElement` means no Dock
icon and no app-switcher entry, so the app *is* the menu bar item, at the
top-right of the screen. Two things open the dashboard from it:

- **Open Dashboard** in the dropdown (⌘D).
- **Double-clicking `Vitals.app`** in Finder while it is already running.
  Without a window this would otherwise be a silent no-op, which reads as a
  broken app; AppKit sends `applicationShouldHandleReopen:` instead and the
  tray answers it by opening the dashboard.

The dashboard opens in a **native window** — a `WKWebView` inside an
`NSWindow`, not a browser tab — 1000×760 the first time, then wherever you
last left it. The window is its own process (`vitals window`, launched by
the tray through LaunchServices and hidden from `--help`): while it is open
it has a Dock tile and an app menu, ⌘W closes it and ⌘Q quits Vitals
entirely, and when it closes the process exits. That is what keeps a closed
dashboard at zero: WebKit's helper processes, the sockets and the page all
belong to the window process and go with it, and the tray never hosts a
web view at all — its footprint is the same 17 MB before, during and after.
If the tray quits or crashes, the window notices (a kqueue watch on the
parent, no polling) and exits within milliseconds. `vitals dashboard` from
a terminal still opens the same page in your default browser.

Either way the server runs **inside the tray process**, not as a child, so
quitting vitals takes the dashboard down with it and nothing is left
orphaned. It is started on demand: a tray that has never been asked for the
dashboard has no listener and no second sampler.

### The dashboard

One page, built to be read top-down:

- **Verdict** first — the same call `vitals pressure` makes, as an icon,
  a label, the one-line summary, the reasons, and the process it blames.
- **Four tiles** — CPU, GPU, memory, power — each a headline figure, a
  status dot, one line of context and a trend line.
- **A 2 / 5 / 15-minute range control** that scopes everything below it.
  The page keeps fifteen minutes of samples, so widening is instant.
- **CPU and GPU over time**, with a crosshair and tooltip (arrow keys step
  through samples once the chart has focus) and the current value labelled
  at the line's end.
- **One bar per core**, efficiency cores in one colour and performance
  cores in another — the same two colours the dropdown uses — labelled
  `E0 … P7` by position, with clock speed on hover.
- **Memory and swap meters** whose fill colour follows the kernel's
  pressure signal. Swap is drawn relative to physical memory, because
  macOS grows the swap file on demand and its own size says little.
- **Top processes** by CPU or by memory, with inline bars.

Every chart has a **Chart / Table** toggle for the same data as a table.
Polling stops whenever the page is hidden, and if the server goes away the
last good render stays on screen, dimmed, until it is back. Light and dark
follow the system.

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
version. The request carries the GitHub API headers, `User-Agent:
vitals/<version>`, and the standard headers macOS's networking adds to
every app's requests (such as your preferred languages) — no account, no
device identifier, no usage data — and nothing is downloaded until you
ask. *Check for Updates…* runs the same check on demand and reports
either way.

When the latest release is newer, an accent-coloured dot appears after the
digits in the menu bar and *Update to Vitals x.y.z…* appears at the top of
the dropdown. *Install and Relaunch* downloads the release's tarball and
`SHA256SUMS`, verifies the hash, unpacks next to the bundle, checks that
the new bundle's version is the release's and — when the running copy is
signed with a Developer ID — that the new one is validly signed by the
same team, then swaps it in and relaunches. Any failure leaves the
installed copy untouched and says why.

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

## Development

```bash
make build   # cargo build --release
make test    # cargo test --workspace
make lint    # cargo clippy --workspace --all-targets -- -D warnings
make fmt     # cargo fmt --all
```

`make lint` is clippy with `-D warnings` — it must be clean, not just
free of hard errors.

The dashboard is a Vite + React app in `dashboard/`. `npm run build` there
writes `dashboard/dist`, which `cargo build` embeds into the binary via
`include_dir!` — rebuild the binary after rebuilding the page. For
iteration, `npm run dev` serves the source with hot reload and proxies
`/api/*` to `127.0.0.1:9876`, so it works against any running `vitals
serve`, including the one inside the tray. `npm run lint` is oxlint.

## Contributing

Bug reports, measurements and pull requests are welcome — see
[CONTRIBUTING.md](CONTRIBUTING.md) for the loop, the layout, and the few
rules that aren't style. Vulnerabilities go through
[SECURITY.md](SECURITY.md), not the issue tracker.

## License

[MIT](LICENSE) © 2026 Bill Sun.
