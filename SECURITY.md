# Security policy

## Supported versions

vitals is pre-1.0. Only the latest release on `main` receives fixes.

## Reporting a vulnerability

Please **do not** open a public issue. Use GitHub's private vulnerability
reporting instead: the **Security** tab of this repository → **Report a
vulnerability**. You will get an acknowledgement within a few days and a fix
or a stated decision within 30.

## What is and isn't in scope

vitals is a local tool. Its threat model is short, and worth knowing before
you report:

- **The server is loopback-only by construction.** `vitals serve` binds
  `127.0.0.1` and rejects any request whose `Host` header is not a loopback
  name — see `crates/app/src/serve/mod.rs` and the exact contract in
  `docs/agents.md`. A way to make it answer a request from another machine,
  or from a browser page on another origin, is a vulnerability.
- **There is no authentication.** Any process running as your user can read
  your machine's metrics from the API. That is by design: the same process
  could read them from the kernel directly. It is not a vulnerability.
- **The binary runs unprivileged.** No `sudo`, no helper tool, no
  entitlements. Anything that would require or escalate to privileges is a
  bug worth reporting.
- **The dashboard is embedded and served from the binary.** It fetches
  only its own relative `/api/*` endpoints. Any way to make it load or
  execute remote content is a vulnerability.
- **Dependencies** are pinned in `Cargo.lock` and
  `dashboard/package-lock.json`; Dependabot watches both. A vulnerable
  dependency should be reported upstream first, then here if it affects
  vitals.

Thank you for taking the time.
