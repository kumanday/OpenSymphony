---
id: OSYM-811
title: Lazy Desktop Launcher Command
milestone: "M10.6: Desktop Run Detail Operations And Interrupts"
priority: 3
estimate: 3
blockedBy: []
blocks: ["OSYM-812"]
areas:
  - cli
  - desktop
  - installer
parent: null
---

## Summary

Add `opensymphony app` with visible alias `opensymphony desktop` as a lazy installer/launcher for the desktop bundle without adding desktop build dependencies to normal Cargo installs.

## Scope

### In scope

- Add CLI command parsing for `opensymphony app` and alias `opensymphony desktop`.
- Materialize or download a versioned desktop bundle into `~/.opensymphony/desktop/<version>/` on first run.
- Verify platform, architecture, version, and checksum before launch.
- Reuse the cached bundle on later runs.
- Provide a repair path for missing, corrupt, or wrong-version cached bundles.
- Preserve `opensymphony run` as the execution-plane entrypoint.

### Out of scope

- Making default `cargo install opensymphony` compile Tauri, npm, or platform desktop dependencies.
- Building a full signed installer.
- Moving scheduling authority into the desktop app.

## Deliverables

- CLI app/desktop command path.
- Versioned desktop cache layout.
- Bundle verification and launch behavior.
- Tests for alias parsing and cache path selection.

## Acceptance Criteria

- [ ] `opensymphony app` and `opensymphony desktop` reach the same launcher flow.
- [ ] A default Cargo install does not compile desktop dependencies.
- [ ] First run materializes or downloads a versioned bundle under `~/.opensymphony/desktop/<version>/`.
- [ ] Later runs launch the cached bundle after verification.
- [ ] Corrupt or mismatched cached bundles produce clear repair guidance.

## Test Plan

- Run CLI parser tests for both commands.
- Run focused unit tests for cache path, version, and platform selection.
- Run a smoke test with a fake or local bundle path if the real desktop bundle is not available.

## Context

- Read `docs/specs/desktop-run-detail-operations-spec.md`.
- Read `docs/installer-and-distribution.md`.
- Inspect CLI command routing in `crates/opensymphony-cli/src/`.
- Inspect desktop app packaging inputs under `apps/desktop/`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep this as a launcher. A real native installer can come later.
