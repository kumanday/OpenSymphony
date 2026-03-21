---
id: OSYM-402
title: Build FrankenTUI operator client
type: feature
area: ui-observability
priority: P1
estimate: 5d
milestone: M4 Operator UX and repo harness
parent: OSYM-400
depends_on:
  - OSYM-401
blocks:
  - OSYM-503
project_context:
  - AGENTS.md
  - README.md
  - docs/ui-frankentui.md
  - docs/repository-layout.md
repo_paths:
  - crates/opensymphony-tui/
definition_of_ready:
  - OSYM-401 is merged
  - FrankenTUI dependency strategy is finalized
---

# OSYM-402: Build FrankenTUI operator client

## Summary
Build the optional terminal operator client using FrankenTUI as a consumer of the control-plane snapshot and update stream.

## Scope
- Create a control-plane client for current snapshot and live updates
- Implement pane-based issue list, issue detail, runtime log, and workspace detail views
- Use FrankenTUI's diff-based rendering and inline-mode strengths for a stable terminal experience
- Support safe read-only navigation in the MVP

## Out of scope
- Daemon mutation controls
- Direct runtime transport access from the UI

## Deliverables
- FrankenTUI application crate
- View reducers and layout state
- Basic keyboard navigation and focus model

## Acceptance criteria
- The UI can render multiple issues and live updates without corrupting terminal state
- The UI remains usable when attached after the daemon is already running
- No UI code depends on orchestrator internals beyond the control-plane contract

## Test plan
- Reducer tests for pane state
- Snapshot-to-view rendering tests where practical
- Manual inline-mode walkthrough
