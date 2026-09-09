# vitals dashboard

The React page `vitals serve` / `vitals dashboard` hands out on localhost.
Polls `GET /api/snapshot` every second, `GET /api/top` and `GET /api/pressure`
every five seconds, and stops polling entirely while the tab is hidden
(`document.visibilityState` / `visibilitychange` — see
`src/hooks/useVisiblePolling.ts`) so a backgrounded or closed tab costs
nothing. No websocket, no SSE, no persistent connection.

`src/types.ts` mirrors `crates/core/src/schema.rs` field-for-field; if the
Rust schema changes, update this file to match.

## Development

```bash
npm install
npm run dev      # dev server with /api/* proxied to a `vitals serve` on :9876
```

## Production build

```bash
npm run build     # -> dist/, embedded into the `vitals` binary via include_dir!
```

Run from the repo root as `make dashboard`, which the Rust `build` and `test`
targets both depend on — `crates/app/src/serve/assets.rs` embeds `dist/` at
compile time, so the Rust side needs a real build in there to serve (or test)
anything beyond the "dashboard not built" placeholder response.
