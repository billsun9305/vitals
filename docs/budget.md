# Measured idle budget

The design's sequencing rule: ship the text-only tray and prove the budget
holds *before* adding custom drawing. If a later change regresses this, the
regression is attributable to that change rather than to everything at once.

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
1 Hz polling, and no measurable CPU at all once idle for 10s and parked.

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
