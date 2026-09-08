# CLAUDE.md snippet

This repo does not (and will not) modify `~/.claude/CLAUDE.md` — that file
configures every Claude Code session on this machine, and only its owner
should change it. If you want future sessions to know about `vitals`
automatically, paste the block below into `~/.claude/CLAUDE.md` yourself.

```markdown
## System metrics

This Mac has `vitals` installed. To answer "why is my machine slow", run
`vitals pressure` (verdict + likely cause), `vitals top -n 5` (what is eating
CPU and memory), or `vitals snapshot` (full SoC state). All output JSON.
Do not use `powermetrics` — it needs sudo and `vitals` already has the data.
```
