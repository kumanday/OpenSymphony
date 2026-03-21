---
id: OSYM-301
title: Implement workspace manager and lifecycle hooks
type: feature
area: workspace
priority: P0
estimate: 4d
milestone: M3 Symphony orchestration core
parent: OSYM-300
depends_on:
  - OSYM-101
  - OSYM-103
blocks:
  - OSYM-204
  - OSYM-304
  - OSYM-305
project_context:
  - AGENTS.md
  - README.md
  - docs/workspace-and-lifecycle.md
  - docs/symphony-spec-alignment.md
repo_paths:
  - crates/opensymphony-workspace/
definition_of_ready:
  - OSYM-101 and OSYM-103 are merged
  - Workspace root policy is agreed
---

# OSYM-301: Implement workspace manager and lifecycle hooks

## Summary
Implement deterministic issue workspace creation, reuse, containment checks, lifecycle hooks, and local manifests under `.opensymphony/`.

## Scope
- Sanitize issue identifiers into stable workspace keys
- Create and reuse issue workspaces under the configured root
- Execute `after_create`, `before_run`, `after_run`, and `before_remove` hooks with timeouts
- Persist issue and run manifests locally for recovery and diagnostics
- Support safe cleanup decisions for terminal issues

## Out of scope
- Remote sandbox provisioning
- Git operations beyond the configured hook commands

## Deliverables
- Workspace manager
- Hook runner
- Local issue manifest format
- Containment and path-safety helpers

## Acceptance criteria
- No workspace path can escape the configured root
- Hooks run in the intended working directory with timeout handling
- Workspace reuse is deterministic for the same issue identifier

## Test plan
- Unit tests for sanitization and containment
- Integration tests for hook execution and timeout
- Cleanup tests for terminal and non-terminal states
