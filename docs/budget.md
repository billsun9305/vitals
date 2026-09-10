# Measured idle budget

The design's sequencing rule: ship the text-only tray and prove the budget
holds *before* adding custom drawing. If a later change regresses this, the
regression is attributable to that change rather than to everything at once.

> **These are the Task 16 figures, kept as the point-in-time baseline they
> were taken as. The binary size below is superseded — Task 19 later embedded
> the dashboard into the executable. For the finished product's numbers see
> [Final figures](#final-figures-after-the-dashboard-was-embedded) at the
> bottom of this file.**

**Machine:** Apple M1 Pro (MacBookPro18,3), macOS 26.5
**Measured:** 2026-09-08, release build (`lto = "fat"`, `opt-level = "s"`,
`strip = true`), menu closed and display awake unless stated otherwise.

| Target | Measured | |
|---|---|---|
| Idle CPU < 0.3% | **0.167%** — 0.10s of CPU in 60s | pass |
| Memory < 25 MB | **15.9 MB** physical footprint (peak 17.2) | pass |
| Binary < 6 MB | **0.87 MB** | pass |
| Threads 3–4 | **5** | see below |
| Parked across display sleep: cumulative CPU essentially unchanged | **0.02s across ~40s asleep** (0.05%), against 0.15% awake | pass |

For comparison, `vitals serve` on the same machine: 0.22s of CPU per 30s of
1 Hz polling of `/api/snapshot`, and effectively nothing once idle for 10s
and parked. See [The server, honestly](#the-server-honestly) below — that
0.22s figure measures one endpoint, not what an open dashboard actually
does.

## Two places the plan's method was wrong

**`vitals top` is the wrong instrument for this.** The plan measured the tray
with `vitals top -n 20 | jq 'select(.name=="vitals")'`. That cannot work, for
two independent reasons, both confirmed against a live tray:

1. It selects by name, and the measuring process is *also* called `vitals`.
   The single row it returned was the `top` invocation itself at 6.3% CPU —
   not the tray. The check would fail on a healthy tray while reporting a
   number describing the wrong process.
2. The tray does not appear in the ranking at all. At ~0% CPU it is nowhere
   near the top 20, so the output is empty — and an empty result asserts
   nothing. Even with (1) fixed, a healthy tray and a missing tray look
   identical.

A ranking tool cannot measure one known process that is supposed to be
invisible. Everything above is instead cumulative CPU time for the tray's own
pid across a fixed window (`ps -o time= -p <pid>`), which is the method
`crates/core/tests/park_cost.rs` already uses via `getrusage`.

**RSS is the wrong memory number on macOS.** `ps -o rss=` reports 42 MB for
the tray, against a 15.9 MB physical footprint. The difference is shared
framework text — AppKit, Foundation, CoreGraphics — which is counted against
every process that links it and is not a cost this app imposes. Physical
footprint (`vmmap --summary`) is what Activity Monitor shows and what
reflects real memory pressure, so it is the number reported above.

Worth knowing, because it applies to the product and not just to this
measurement: `vitals top`'s `mem_mb` is RSS (`crates/core/src/procs.rs:123`,
from `sysinfo`'s `memory()`). For a framework-linked GUI process it will read
roughly 2.5x higher than Activity Monitor does, and summing `mem_mb` across
processes can exceed physical RAM, because shared pages are counted once per
process. That is the same convention `ps` uses and it is defensible, but it
is not what a user comparing against Activity Monitor expects.

## Notes

**Threads: 5, not the 3–4 the plan guessed.** Stable across five samples
taken two seconds apart (a freshly launched process shows 4 for a moment).
The plan's figure was an estimate, not a requirement, and nothing here is
load-bearing on it. Incidentally this cross-validates the process table:
`vitals`' own `threads` field, which comes from `libproc`'s `pti_threadnum`,
agreed with `ps -M` on all five samples.

**The park is real, and so is the notification path.** Instrumenting the
workspace handlers and running `pmset displaysleepnow` printed
`is_main=true` for both the sleep and the wake, confirming NSWorkspace
delivers on the main thread — an assumption those handlers rely on that had
never been checked, and the same kind of assumption that caused this
project's one main-thread bug. Across the sleep window the tray used 0.02s
of CPU in ~40 seconds, about a third the per-second rate of the awake tray.

**Go for Task 17.** Every budget row passes, so the custom drawn panel can
proceed. If it regresses any row, this file is the baseline to diff against.

## Task 17 re-measurement: the custom-drawn panel

> Also a point-in-time reading, taken before the dashboard was embedded. Its
> binary-size row is superseded by [Final figures](#final-figures-after-the-dashboard-was-embedded).

Same machine and method as above, release build, menu **closed** for the
entire run (the panel view exists but is never mounted on screen, since it is
only added to the window when the dropdown opens). `vitals` was launched and
left alone; `ps -o time= -p <pid>` was read once shortly after launch and
again after it had run for a further ~212s, so the delta excludes the
few-hundred-millisecond startup burst (process creation, `Sampler::new`,
`NSMenu`/`NSStatusItem`/`PanelView` construction) that a from-zero reading
would otherwise fold into a strict-idle number.

| Target | Before (Task 16) | After (Task 17) | |
|---|---|---|---|
| Idle CPU < 0.3% | 0.167% | **~0.19%** — 0.40s of CPU over 212s, menu closed | pass |
| Memory < 25 MB | 15.9 MB physical footprint (peak 17.2) | **16.1 MB** physical footprint (peak 17.3) | pass |
| Binary < 6 MB | 0.87 MB | **0.87 MB** (910,432 bytes) — unchanged | pass |
| Threads 3–4 (baseline observed 5) | 5 | **5** | unchanged |

The small CPU and footprint deltas are consistent with what a closed-menu run
should cost: one extra `NSMenuItem`, one extra `NSView` subclass instance and
its `RefCell<PanelState>` (an empty `Vec<f32>` and a `Ring<f32>` of capacity
60, none of it populated while the menu is closed), and nothing on the poll
path changed. `drawRect:` is never invoked and `PanelView::update` is never
called while `menu_open` is false, so the panel adds a fixed, one-time
allocation cost and no recurring one. Both readings land within the
noise band of the Task 16 baseline: `ps`'s CPU-time field is quantized to
centiseconds, so a ~0.02–0.03 percentage-point wobble at this magnitude is
expected sampling noise rather than a real regression, and it is an order of
magnitude below the 0.3% ceiling either way.

Not re-measured by that pass: the parked-across-display-sleep number
(unchanged code path; Task 17 touches nothing on it).

### Menu open, measured

The menu-open case was left open above as needing "real AppKit UI ... which
this environment cannot do". It can be driven, and it matters more than the
closed case: an open dropdown is the only state that both samples at 1 Hz and
draws, and the run loop is in `NSEventTrackingRunLoopMode` throughout — the
mode in which a default-mode timer silently stops firing, which is a bug this
project has already shipped once.

Harness: two one-shot `NSTimer`s registered in `NSRunLoopCommonModes`, the
first calling `performClick` on the status item button to open the menu, the
second calling `cancelTracking` 30s later, with a counter incremented in
`drawRect:`. Measured on the release build, then removed.

| | measured |
|---|---|
| Redraws while the menu was open ~30s | **30** — i.e. 1 Hz, exactly the menu-open cadence |
| CPU, menu open, 26s | **1.04%** |
| Redraws in the 60s *after* the menu closed | **0** |
| CPU, settled, menu closed, 58s | **0.190%** (baseline 0.167%) |

Three things this establishes that reasoning could not. The panel really does
update while the menu is open, so the common-modes timer registration is
doing its job in tracking mode. The panel really does stop when the menu
closes — zero redraws in a full minute, which is the claim the section above
makes on inspection alone. And the cost returns to the idle baseline
afterwards rather than staying elevated, so nothing is left running.

An open dropdown costs about 6x idle. That is the intended trade and it is
bounded by how long a user holds the menu open, but it is the number to watch
if the panel ever gains animation: at 1 Hz the drawing is nearly free next to
the sampling, and that stops being true if redraws are decoupled from
samples.

## Final figures, after the dashboard was embedded

The numbers above were taken before Task 19 embedded the built React
dashboard into the binary via `include_dir!`, so the 0.87 MB binary figure
is superseded. Re-measured on the finished product, same machine and method:

| Target | Task 16 | Final | |
|---|---|---|---|
| Idle CPU < 0.3% | 0.167% | **0.183%** — 0.11s over 60s | pass |
| Memory < 25 MB | 15.9 MB | **17.2 MB** physical footprint | pass |
| Binary < 6 MB | 0.87 MB | **1.18 MB** (1,175,040 bytes) | pass |

The binary grew by ~200 KB, which is the gzip-era cost of carrying the whole
dashboard — HTML, CSS and a 209 KB JS bundle — inside the executable so that
`vitals serve` has no runtime dependency on a build directory. Still a sixth
of the ceiling. Idle CPU and footprint moved within noise; neither the
dashboard's assets nor the panel cost anything while nothing is looking at
them, which is the property the whole design is built around.

## The server, honestly

Two corrections to the `vitals serve` line near the top of this file, both
found by the final whole-branch review.

**"0.22s per 30s" measured only `/api/snapshot`.** The shipped dashboard also
polls `/api/top` and `/api/pressure` every 5 seconds, and each of those walks
the whole process table. Measured on the real poll pattern:

| | CPU per 30s of dashboard polling |
|---|---|
| As first written | **2.09s (~7%)** |
| After the review's fixes | **0.93s (~3.1%)** |

The bulk of that saving was `/api/pressure` standing up a second
`macmon::Sampler` on every request — about half a second of setup — and
sampling the SoC concurrently with the server's own background worker. It now
evaluates from the snapshot the worker already cached. What remains is two
process-table enumerations per 5-second tick, which is the honest cost of the
questions being asked. An open dashboard is an active state; the design
promise is that a *closed* one costs nothing, and that still holds.

**"No measurable CPU at all" was slightly too strong.** The server's cache
thread wakes every 250ms regardless, so a parked, idle server still costs
about 0.01s per 60s. That is 0.017%, far below anything that matters, but it
is not zero, and it is the one loop in the product that never parks.

## Idle cost depends on how busy the machine is

The same installed binary, measured twice:

| Conditions | Idle CPU |
|---|---|
| Quiet machine | **0.183%** over 60s |
| Load average 55-84 | **0.267%** over 180s |

Both pass the 0.3% ceiling, but the second is close to it, and no code
changed between them. Sampling costs more per unit of work on a contended
machine — more context switches, colder caches. Worth stating plainly because
it cuts against the grain of the measurement: the tool is at its most
expensive exactly when the machine is already struggling, which is exactly
when someone is looking at it. Every other figure in this file was taken on a
relatively quiet machine and should be read as a floor rather than a
guarantee.

## The tray can now host the dashboard, and that changes what "idle" means

The tray's "Open Dashboard" item (and the Finder-reopen handler behind a
double-click) starts `serve::run` on a thread **inside the tray process**
rather than spawning a child. That keeps the lifetime honest — quitting
vitals takes the dashboard with it — but it means the tray process has two
states worth measuring, not one.

| State | Measured |
|---|---|
| Never asked for the dashboard: no listener, one sampler | **0.200%** — 0.24s over 120s |
| Dashboard open in Chrome, polling at 1 Hz / 5 s | **3.68%** — 2.21s over 60s |

Both taken on the same build at load average 37-69, which is why the idle
figure sits above the 0.183% quiet-machine baseline and below the 0.267%
measured at load 55-84 — it is the same load-dependence documented above,
not a regression from this feature.

Two things the first row is deliberately making a claim about. The listener
count is zero until someone asks, so the on-demand path really is on-demand:
a tray that is only ever a tray pays nothing for the dashboard's existence.
And thread count returns to 5, the same as before this feature — an
8-thread reading taken in the first 60 seconds after launch is the startup
transient, not the steady state, the same way the very first CPU window is.

The second row is not a regression either: it is the cost of an open
dashboard, which [The server, honestly](#the-server-honestly) already prices
at ~3.1% for a standalone `vitals serve` under load. Folding the server into
the tray did not make it cheaper or dearer; it moved where it lives.

## The dashboard window, and why it is a separate process

The first native window (`WKWebView` inside the tray process) was measured
on the installed build, `footprint -p` physical footprint, after opening
the dashboard once and closing it with ⌘W:

| | Before ever opening | Window open | After close |
|---|---|---|---|
| Tray process | 17 MB | 36 MB | **43 MB** |
| `com.apple.WebKit.GPU` | — | 27 MB | **13 MB, still running** |
| `com.apple.WebKit.Networking` | — | 7 MB | **6 MB, still running, three keep-alive sockets to :9876 still open** |
| `com.apple.WebKit.WebContent` | — | 77 MB | exited |
| Tray CPU | 0.2% | 3.6% | 0.1% |

Tearing the `WKWebView` down on close (rather than navigating it to
`about:blank`) did free the WebContent process, but nothing else: the
Networking and GPU helpers are per-app singletons that no public API ends,
and WebKit's UI-process side stays loaded in whichever process created a
web view. Net: a tray that had shown the dashboard once carried ~62 MB
more than one that never had, for the rest of its life. That is the one
number this product exists to keep small, so the window moved out of the
tray into a `vitals window` child process that LaunchServices launches
and that exits when its window closes. Its measurements will be added
here when it lands.

