---
id: OSYM-724
title: Runtime Timeline And Terminal/Log Association
milestone: "M3: Task Graph Operations And OpenHands Run UI"
priority: 2
estimate: 8
blockedBy: ["OSYM-704", "OSYM-713", "OSYM-723"]
blocks: ["OSYM-725", "OSYM-770", "OSYM-771"]
parent: null
---

## Summary

Build the runtime timeline and associate terminal/log output with runs, commands, issues, and sub-issues.

## Scope

### In scope

- Group related runtime events.
- Summarize tool calls and command activity.
- Link timeline events to files, commands, logs, terminal panes, diffs, and task graph nodes.
- Associate terminal/log streams with run, workspace, command, issue, and sub-issue.
- Expose scrollback reads, live stream frames, search, and jump-to-event.

### Out of scope

- Approval decision actions.
- Codex event mapping.
- Hosted admin monitoring.

## Deliverables

- Timeline UI.
- Event grouping tests.
- Terminal/log service endpoints.
- Frontend terminal/log integration.

## Acceptance Criteria

- [ ] Users can inspect runtime events as grouped, readable timeline entries.
- [ ] Terminal/log frames can be traced back to run and task graph context.
- [ ] Scrollback reads and live frames remain consistent after reconnect.

## Test Plan

- Run timeline grouping tests with normalized OpenHands event fixtures.
- Run terminal/log association tests for multiple commands and reconnect cases.

## Context

- Source sections: `docs/host-client-architecture.md` 4.8 and 5.4.
- This task builds on the terminal renderer prototype and event journal replay behavior.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The timeline should help users understand active work and failure causes quickly.
