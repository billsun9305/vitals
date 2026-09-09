# vitals

Apple Silicon system monitor. The CLI is a single Rust binary that samples
CPU, GPU, memory, power, and thermal state in-process — no `powermetrics`,
no `sudo`, no subprocess of any kind — and prints structured JSON an agent
(or a script, or you) can consume directly.

Today `vitals` is a CLI with four verbs: `snapshot`, `top`, `pressure`,
`watch` (plus `serve`/`dashboard`, a local web dashboard — see
`vitals --help`). The same binary, run with no arguments, is also a menu
bar tray item; see [below](#the-menu-bar-app) for what it does and how to
install it as a login item.

## Install

Two independent ways to install, depending on what you want: the CLI on
its own, or the menu bar app with autostart. Both end with a `vitals` on
`PATH`, and it's fine to use either first — `make install-app` simply
repoints the same symlink at the app bundle's copy of the binary instead
of the raw build output.

### CLI only

```bash
make build    # cargo build --release -> target/release/vitals
make install  # symlinks target/release/vitals into $PREFIX/bin (default /usr/local/bin)
```

`make install` needs write access to `$PREFIX/bin`, which usually means
`sudo make install`. Override the prefix with `make install PREFIX=$HOME/.local`
if you'd rather not use sudo. `make uninstall` removes the symlink.

Once installed, `vitals` is on `PATH` and every command below works from
anywhere.

### Menu bar app, with autostart

```bash
make install-app  # builds, bundles, copies to /Applications, registers the
                   # LaunchAgent, and symlinks $PREFIX/bin/vitals to it
```

This copies `dist/Vitals.app` to `/Applications/Vitals.app`, installs
`resources/com.billsun.vitals.plist` into `~/Library/LaunchAgents/` so the
tray starts at every login, and (re)creates the `$PREFIX/bin/vitals`
symlink, now pointing at `/Applications/Vitals.app/Contents/MacOS/vitals`.
`make uninstall-app` reverses all of it (`launchctl unload`, remove the
LaunchAgent, remove `/Applications/Vitals.app`, remove the symlink).

`make install-app` needs `sudo` under the same condition as plain
`make install` (write access to `$PREFIX/bin`), plus write access to
`/Applications` and `~/Library/LaunchAgents`, which do not need `sudo`.

**The app is ad-hoc signed, not notarized.** `scripts/bundle.sh` runs
`codesign --sign -`, which satisfies `codesign --verify` and lets the
binary run, but it is not a real Developer ID signature — there's no team
identity behind it, and it will never pass notarization
(`spctl -a --type execute` on the built bundle reports it as rejected).
Launching it via `launchctl`/the LaunchAgent execs the binary directly and
is unaffected by this. But if you ever double-click `Vitals.app` in
Finder (or otherwise open it through Launch Services) and macOS calls it
"unidentified" and refuses to open it, right-click → Open once to trust
it, or approve it under System Settings → Privacy & Security. This is
expected and permanent for a locally-built, non-distributed app — it is
not a bug in the bundle, and it is why the release note below does not
claim notarization.

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
showing live text plus a custom-drawn dropdown (per-core bars, a CPU
sparkline) — no continuous graph redraw while the dropdown is closed, and
the sampling cadence backs off automatically while the display sleeps.
See `docs/budget.md` for the measured idle cost.

`make install-app` packages this into `dist/Vitals.app`
(`scripts/bundle.sh` plus `resources/Info.plist`, which sets
`LSUIElement` so the app never shows a Dock icon or an app-switcher
entry), installs it to `/Applications`, and registers
`resources/com.billsun.vitals.plist` as a per-user LaunchAgent so the
tray starts at every login — see [Install](#install) above for the exact
targets. `make uninstall-app` removes all of it: the LaunchAgent (after
`launchctl unload`), `/Applications/Vitals.app`, and the `$PREFIX/bin/vitals`
symlink.

A LaunchAgent rather than `SMAppService`: no entitlements to declare, the
plist is trivially inspectable (`plutil -lint`,
`cat ~/Library/LaunchAgents/com.billsun.vitals.plist`), and
`launchctl unload`/`load` round-trips cleanly while debugging, versus
`SMAppService`'s more opaque registration.

### Releasing a new version

Not automated by this repo, and not run as part of building or installing
it — recorded here for whoever cuts the next tag:

```bash
git tag v0.1.0
git push --follow-tags
```

Bump `CFBundleShortVersionString` in `resources/Info.plist` and
`workspace.package.version` in `Cargo.toml` together before tagging; they
are not currently derived from each other.

## Development

```bash
make build   # cargo build --release
make test    # cargo test --workspace
make lint    # cargo clippy --workspace --all-targets -- -D warnings
make fmt     # cargo fmt --all
```

`make lint` is clippy with `-D warnings` — it must be clean, not just
free of hard errors.
