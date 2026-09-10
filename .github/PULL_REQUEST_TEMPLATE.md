## What

<!-- One paragraph: what changes, and why. Link the issue if there is one. -->

## How it was verified

<!-- The commands you ran and what they said. For anything under
     crates/app/src/tray or crates/app/src/serve, include the measurement
     docs/budget.md asks for. For anything visual, a screenshot or the
     render_panel PNGs. -->

- [ ] `make fmt` · `make lint` · `make test` are clean
- [ ] `docs/agents.md` updated if JSON output changed (and `schema_version` bumped)
- [ ] `docs/budget.md` updated if a measured number moved
- [ ] `CHANGELOG.md` has an entry under *Unreleased*
