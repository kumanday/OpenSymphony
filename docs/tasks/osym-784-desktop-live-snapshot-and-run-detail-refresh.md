---
id: OSYM-784
title: Desktop Live Snapshot And Run Detail Refresh
milestone: "M10.4: Desktop Live Operations And Model Polish"
priority: 2
estimate: 3
blockedBy: []
blocks: []
areas:
  - desktop
  - gateway
  - ui
parent: null
---

## Summary

Make the desktop app update from live gateway snapshots and run evidence changes without requiring an app restart.

## Scope

### In scope

- Wire the shared app shell to consume the existing gateway event stream exposed through `GatewayTransport.events()`.
- Keep the desktop path on the loopback HTTP/SSE transport; do not depend on the unimplemented Tauri channel event stream.
- Refresh the dashboard snapshot and task graph when the gateway publishes a newer snapshot.
- Refresh the selected run detail, changed files, selected diff, activity, validation evidence, and approvals when the selected run changes.
- Avoid noisy full reloads when events do not affect the current project or selected run.
- Add the smallest regression coverage proving a live event updates changed files or run detail without a full app restart.

### Out of scope

- Replacing the control-plane event contract.
- Implementing Tauri channel streaming.
- Reworking the desktop visual layout.

## Deliverables

- Live event subscription in the shared UI shell.
- Selected-run refresh logic for run detail and evidence panels.
- Regression coverage for live changed-file/run-detail updates.
- Operations or desktop docs note if operator-visible behavior changes.

## Acceptance Criteria

- [ ] With a running issue selected, changed files appear in the desktop Run Detail panel after the gateway publishes the update, without restarting the desktop app.
- [ ] The task graph and selected run status follow newer control-plane snapshots.
- [ ] The implementation cleans up event subscriptions when the shell is torn down or the gateway URL changes.
- [ ] The desktop still works when the event stream is unavailable by falling back to the existing one-shot refresh behavior.

## Test Plan

- Run the affected TypeScript UI/client tests.
- Run a focused desktop or app-shell regression test using a fake transport event stream.
- Manually verify against a local `opensymphony run` session that desktop changed files match the TUI after a live update.

## Context

- The backend already returns current files from `/api/v1/runs/{run_id}/files`.
- The TUI consumes `ControlPlaneClient::stream_updates()`, while the desktop app shell currently performs one-shot loads for snapshots and run detail.
- `apps/desktop/src-tauri/src/commands.rs` explicitly leaves Tauri channel event streams unavailable, so the intended near-term path is the loopback HTTP/SSE transport.

## Definition of Ready

- [ ] Current desktop/TUI live-update behavior is reproduced or described.
- [ ] Required source files and tests are identified.
- [ ] A coding agent can start from this task without additional planning context.
