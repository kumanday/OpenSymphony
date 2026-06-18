---
id: OSYM-843
title: Tauri Debug-In-Zed Launch Action
milestone: "M12.5: ACP Debugging And IDE Attach"
priority: 3
estimate: 5
blockedBy: ["OSYM-840", "OSYM-842"]
blocks: ["OSYM-844", "OSYM-845"]
areas:
  - debugging
  - desktop
  - tauri
parent: null
---

## Summary

Wire the desktop app debug action to resolve an issue key to the exact issue workspace path and open Zed on that workspace.

## Scope

### In scope

- Add a Tauri or gateway command that resolves an issue key to its exact OpenSymphony issue workspace path.
- Launch `zed -n <workspace-path>` where available.
- Show concise instruction text telling the operator to start the OpenSymphony Debug external agent in Zed.
- Handle missing Zed, missing workspace, and missing manifests with recoverable UI feedback.

### Out of scope

- Auto-starting Zed agent threads.
- Creating additional workspace debug manifests.

## Deliverables

- Desktop debug action plumbing.
- Workspace path resolution API or command.
- UI states for launch success, missing editor, and invalid workspace.

## Acceptance Criteria

- [ ] Tauri can open Zed on the exact issue workspace root for a selected issue.
- [ ] The app does not open target repo roots or OpenHands conversation store directories for ACP debug.
- [ ] The UI leaves rich orchestration and visualization in OpenSymphony while Zed owns code inspection and manual edits.
- [ ] Missing editor or invalid workspace states include actionable recovery text.

## Test Plan

- Add unit tests for workspace resolution and command payloads.
- Add desktop integration tests or manual verification for the launch command where CI permits.
- Run frontend tests for debug action UI states.

## Context

- Builds on OSYM-840 and OSYM-842.
- Read `docs/opensymphony-acp-debugging-spec.md` Tauri integration and workspace selection sections.
- Existing desktop action surfaces live near the shared client and Tauri shell code.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

OpenSymphony owns workspace resolution; Zed owns code and agent thread UI.
