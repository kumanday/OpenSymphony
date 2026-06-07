---
id: OSYM-718
title: Desktop Alpha Recovery
milestone: "M7: Shared Client And Desktop Alpha"
priority: 2
estimate: 5
blockedBy: ["OSYM-711", "OSYM-712", "OSYM-714", "OSYM-715", "OSYM-717"]
blocks: ["OSYM-770", "OSYM-771"]
parent: null
linear: "COE-449"
---

## Summary

Recover the M7 desktop-alpha outcome by replacing remaining web/desktop stubs with a launchable Tauri desktop app that mounts the shared OpenSymphony UI, can use local gateway/profile data, and proves the top-level desktop flow with a smoke test.

## Scope

### In scope

- Mount the real shared app shell in both the desktop and web entrypoints instead of the generated stub or placeholder renderer.
- Provide a usable read-only desktop alpha flow for dashboard, task graph, run detail, profile/connection status, and stream-health state using the M7 gateway/client contracts.
- Persist and list desktop connection profiles so profile selection survives app restart and does not return synthetic empty state.
- Fix API-client route or capability mismatches that prevent frontend transports from talking to current gateway endpoints.
- Wire the desktop transport to real Tauri commands/events where available, and fall back to loopback HTTP/event streams when native channels are unavailable.
- Make unavailable capabilities visibly unavailable instead of advertising stubbed native behavior.
- Add automated smoke/contract coverage that proves the desktop entrypoint is not the stub page.
- Update focused docs when operator run/build instructions or capability expectations change.

### Out of scope

- Final visual polish beyond making the alpha interface usable and coherent.
- Full editable task graph mutations.
- Full terminal emulator behavior beyond existing terminal/log renderer support.
- Hosted browser authentication and remote WSS behavior covered by later milestones.
- Production release signing, notarization, installer packaging, and full accessibility/security release pass.

## Deliverables

- Desktop and web entrypoints that mount the real shared frontend.
- Desktop profile persistence/listing and selected-profile connection behavior.
- Working dashboard/task graph/run-detail read flow against gateway fixtures or a local gateway.
- Desktop transport fallback behavior with truthful capability reporting.
- Top-level UI smoke tests and transport/route contract tests.
- Short verification notes showing how to run the desktop alpha locally.

## Acceptance Criteria

- [ ] Building or launching the desktop app renders the OpenSymphony app shell, not a generated stub page or placeholder-only renderer.
- [ ] Users can navigate from dashboard to task graph to a run detail view using fixture or live gateway data.
- [ ] Desktop connection profiles can be created, listed, selected, persisted, and reloaded after process restart.
- [ ] The selected profile drives gateway reads and stream subscription behavior, with loopback fallback when native desktop channels are unavailable.
- [ ] The client uses endpoint paths that match the current Rust gateway routes.
- [ ] Capability discovery does not advertise stubbed native behavior as available.
- [ ] Automated tests fail if the desktop entrypoint regresses to the stub page or if the primary app shell is not mounted.
- [ ] Verification evidence includes frontend tests, desktop build/type-check, Rust desktop tests, and a smoke artifact or command proving the app shell renders.

## Test Plan

- Run `npm test`.
- Run `npm run type-check`.
- Run `npm run build --workspace=@opensymphony/desktop`.
- Run focused desktop/Tauri Rust tests under `apps/desktop/src-tauri`.
- Run an app-shell smoke test against fixtures or a fake gateway.
- Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and focused Rust gateway/desktop tests for touched crates.
- Run `git diff --check`.

## Context

- Linear issue: [COE-449](https://linear.app/trilogy-ai-coe/issue/COE-449/desktop-alpha-recovery-replace-stubs-with-functional-app).
- This is a corrective M7 task created after post-mortem review found that COE-398, COE-402, COE-404, and COE-410 left top-level desktop functionality stubbed or insufficiently integrated.
- M8 and later tasks still own rich run UI, browser-hosted transport, hosted auth, and release hardening; this task owns the missing desktop-alpha baseline.
- The orchestrator should not auto-pick this work while it is being handled manually, so the Linear issue is intentionally in Backlog.

## Definition of Ready

- [ ] COE-397, COE-398, COE-402, COE-404, and COE-410 have landed or their current contracts are visible in `main`.
- [ ] The Linear issue remains in Backlog until a human intentionally moves it for orchestrator execution.
- [ ] A manual implementer can begin from `main` without waiting for later M8/M10/M13 work.

## Notes

Do not accept scaffolding as completion unless the acceptance criterion explicitly says scaffold. The desktop alpha should be modest, but it must be real enough to launch and inspect OpenSymphony state.

## Verification Evidence

- Desktop app shell screenshot: [COE-449 desktop alpha](../images/coe-449-desktop-alpha.png)

## Linear Dependencies

- Blocked by: COE-397, COE-398, COE-402, COE-404, COE-410
- Blocks: COE-430, COE-431, release-readiness decisions for the desktop app

## Linear Metadata

- Planning wave: rich-client-hosted-mode-recovery
- Milestone: M7: Shared Client And Desktop Alpha
- Priority: High
- Estimate: 5
- Initial status: Backlog

## Definition of Done

- All acceptance criteria above are satisfied.
- Relevant tests pass, or manual verification evidence is attached.
- A PR implementing this issue is opened, linked back to Linear, and reviewed by the automated PR reviewer.
