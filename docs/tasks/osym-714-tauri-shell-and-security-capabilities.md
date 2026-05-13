---
id: OSYM-714
title: Tauri Shell And Security Capabilities
milestone: "M2: Shared Client And Desktop Alpha"
priority: 2
estimate: 5
blockedBy: ["OSYM-710"]
blocks: ["OSYM-715", "OSYM-716", "OSYM-717"]
parent: null
---

## Summary

Create the Tauri desktop wrapper and configure explicit native capability boundaries.

## Scope

### In scope

- Create the Tauri project wrapper and mount the shared frontend.
- Configure development and production builds.
- Add app metadata and icons.
- Define Tauri capabilities by window and command.
- Scope file/folder selection, notification, settings, and local process supervision permissions.

### Out of scope

- Daemon discovery and supervision behavior.
- Keychain-backed credential storage.
- High-throughput local stream optimization.

## Deliverables

- Desktop app skeleton.
- Tauri build scripts.
- Capability files.
- Tauri security checklist.

## Acceptance Criteria

- [ ] The desktop app builds and loads the shared frontend in development mode.
- [ ] Frontend-accessible native commands are explicitly scoped.
- [ ] Security-sensitive capabilities have test or review coverage.

## Test Plan

- Run the Tauri development build and production build commands.
- Run capability configuration tests or static checks available in the chosen Tauri setup.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.2.1 and `docs/host-client-implementation_plan.md` P0.5 and P3.1/P3.5.
- Desktop is the premium local experience and should still connect to hosted remote profiles.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep privileged commands in the Rust shell with narrow request and response types.
