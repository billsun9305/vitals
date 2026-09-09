# Using vitals from an agent

Four commands, JSON by default, no MCP server needed. `snapshot`, `top`,
and `pressure` are each one shot: they sample, print, and exit, so they
cost nothing when you are not asking. `watch` is the one exception — it
streams until you stop it.

| Question | Command |
|---|---|
| What is the machine doing right now? | `vitals snapshot` |
| What is eating CPU or memory? | `vitals top -n 5` |
| Is anything wrong, and what caused it? | `vitals pressure` |
| I need a running series, not one point. | `vitals watch -i 2` |

Notes that prevent misreads:

- `cpu_pct` in `snapshot` is **utilization**; `cpu_scaled_pct` is the same
  residency weighted by clock speed. They differ by 3x on an idle machine.
- `cpu_pct` in `top` is **per process** and exceeds 100 on multi-core work,
  exactly like `ps`.
- All sizes are MB, all power is watts, all temperatures are Celsius — the
  unit is always in the key name.
- `pressure` reports `history: false` when it has no recent baseline; the
  swap-trend reasons are simply absent, and every other rule still applied.
  Verified on this machine: deleting `~/.cache/vitals/last.json` and
  re-running `vitals pressure` produces `"history": false` with the rest of
  the verdict unchanged.
- Prefer `pressure` over reasoning from raw numbers. Its `summary` is a
  sentence you can quote directly.
- `pressure` exits `0` by default even when `state` is `warning` or
  `critical` — it is reporting, not failing. Pass `--exit-code` to fold the
  verdict into the exit status instead (`0` nominal, `3` warning, `4`
  critical). A runtime failure exits `1` either way, and a malformed
  invocation exits `2` from the argument parser before the command runs —
  so with `--exit-code` the status alone tells you which of the three
  happened: the machine is unwell (3/4), the tool broke (1), or you called
  it wrong (2). Without `--exit-code` a nonzero status can only mean the
  latter two.
- `watch` prints one compact (not pretty-printed) JSON object per line and
  flushes after every line, so a consumer piping it — `vitals watch | head
  -1`, a log tail, a streaming parser — sees each sample as soon as it is
  written and the process exits cleanly (status 0) when the reader closes
  the pipe.
- `top` lists **every** process including `vitals` itself, and the figure it
  reports for itself is inflated: enumerating the process table is the
  busiest this tool ever gets, and it happens inside the very window it is
  measuring, so a `vitals top` run can show `vitals` near the top at tens of
  percent. It idles at 0.167% (see `docs/budget.md`). Ignore that row. The
  `pressure` verdict already does — it filters the measuring process out of
  its suspects, so `summary` never blames the tool for the load.
- `top`'s `sample_ms` is `200` by default — the window `sysinfo` measures
  per-process CPU delta over. It is not configurable per invocation today.

## Example: an unhealthy machine

Real output from this machine while it was genuinely under memory
pressure and CPU load (see the README for the full transcript of all four
verbs):

```json
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

An agent answering "why is my Mac slow" can quote `summary` directly, or
walk `reasons`/`suspects` for a structured answer — no arithmetic on raw
`snapshot` fields required.
