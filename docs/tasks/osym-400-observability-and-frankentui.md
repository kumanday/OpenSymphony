---
id: OSYM-400
title: Observability and FrankenTUI
type: parent
area: ui-observability
priority: P1
estimate: 2w
milestone: M4 Operator UX and repo harness
depends_on:
  - OSYM-300
blocks:
  - OSYM-500
children:
  - OSYM-401
  - OSYM-402
project_context:
  - AGENTS.md
  - README.md
  - docs/ui-frankentui.md
  - docs/architecture.md
  - docs/implementation-plan.md
repo_paths:
  - crates/opensymphony-control/
  - crates/opensymphony-tui/
definition_of_ready:
  - M3 is merged
  - Snapshot model is agreed
  - FrankenTUI workspace dependency strategy is documented
---

# OSYM-400: Observability and FrankenTUI

## Summary
Add a stable local control plane and the optional terminal UI. The daemon remains correct without the UI.

## Scope
- Read-only control-plane API
- Snapshot derivation and publication
- FrankenTUI operator client with live event updates

## Out of scope
- Browser UI
- Daemon mutation from UI in the MVP

## Child issues
- OSYM-401
- OSYM-402

## Deliverables
- Control-plane server
- Control-plane client models
- FrankenTUI app

## Acceptance criteria
- All child issues are merged
- UI can attach to a running daemon and render meaningful issue state
- Detaching the UI has no effect on daemon correctness

## Test plan
- Snapshot serialization tests
- Reducer tests for TUI state
- Manual operator walkthrough
