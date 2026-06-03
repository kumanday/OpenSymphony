# Tauri Security Checklist

> Maintained alongside the desktop Tauri wrapper (`apps/desktop/src-tauri/`).

## Configuration

- [x] `tauri.conf.json` `identifier` uses a reverse-DNS style ID (`dev.opensymphony.app`).
- [x] CSP is defined; `default-src` is locked to `'self'`.
- [x] `connect-src` allows `localhost:*` for local daemon and remote profile endpoints.
- [x] `dev-csp` is not overridden in dev — production CSP applies to both modes.
- [x] `dangerousDisableAssetCSPModification` is omitted (defaults to `false`).

## Capabilities

| Capability | Windows | Permissions | Risk |
|---|---|---|---|
| `default` | `main` | Core window/app/event/webview | Low |
| `file-selection` | `main` | `dialog:allow-open`, `dialog:allow-save` | Low — user-initiated |
| `notification` | `main` | `notification:allow-show`, `notification:allow-request-permission` | Low |
| `settings` | `main` | `fs:allow-read-text-file`, `fs:allow-write-text-file` | Low — scoped to `$HOME/.config/opensymphony` |
| `process-supervision` | `main` | `shell:default` | **Low** — `shell:default` only grants minimal shell plugin baseline. Stub only; COE-404 will add whitelisted executables and PID tracking |

## Commands

- [x] Every command uses narrow, strongly-typed request and response structs.
- [x] Custom commands are registered via `invoke_handler` macro — no ad-hoc handlers.
- [x] No command returns raw OS-level error details to the frontend.
- [x] File-system access is scoped to `$HOME/.config/opensymphony/**` via the `fs` plugin config.
- [ ] Secrets are never stored in plain-text settings (deferred to COE-409 keychain work).

## Build & Runtime

- [x] No `unsafe` blocks in `apps/desktop/src-tauri/`.
- [x] `rust-version = "1.93"` enforced in `Cargo.toml`.
- [x] Workspace lints forbid `unsafe_code`, warn on `unwrap_used` and `todo`.
- [x] Placeholder icons are present; real icons will replace before release.
- [x] `beforeDevCommand` and `beforeBuildCommand` point to the shared frontend workspace.
- [ ] Production builds ship a stub HTML until the real frontend is mounted (deferred to COE-394).

## Audit Notes

- `process-supervision` capability grants only `shell:default` (minimal shell plugin baseline). No `shell:execute` or `shell:kill` permissions are active. COE-404 must implement:
  - Whitelisted executable paths.
  - PID tracking to prevent arbitrary process kills.
  - Input sanitization on any shell command arguments.
- `settings` capability grants only `fs:allow-read-text-file` and `fs:allow-write-text-file` — no `fs:default` baseline, no directory listing, copy, or binary access. File-system scope is `$HOME/.config/opensymphony/**`. Settings must not accept paths that escape this scope.
- `connect-src` CSP restricts WebSocket connections to pinned hosts `wss://api.opensymphony.dev` and `wss://api.opensymphony.app`. Local daemon traffic uses `ws://localhost:*`.
- Current Tauri `2.11.2` pulls the Linux GTK3 Rust bindings through
  `tauri -> wry/webkit2gtk/gtk`. RustSec `RUSTSEC-2024-0429` fixes the
  `glib::VariantStrIter` advisory in `glib >=0.20.0`, but the current GTK3
  crate requires `glib ^0.18`. Do not force a direct `glib` override; it would
  mix incompatible GTK binding generations.
- `cargo audit --file apps/desktop/src-tauri/Cargo.lock --json` reports
  `vulnerabilities.found = false`, while `cargo audit --deny warnings` still
  fails on all-target informational warnings from the Linux GTK3 stack and
  Tauri `urlpattern`'s transitive `unic-*` crates. Track an upstream
  Tauri/wry/GTK migration before treating the Linux desktop dependency audit as
  warning-clean.
