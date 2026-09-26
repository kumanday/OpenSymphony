---
id: OSYM-843
title: Multi-Harness Desktop Debug-In-Zed Action
milestone: 'M13: ACP Debugging And IDE Attach'
priority: 3
estimate: 5
blockedBy:
- OSYM-840
- OSYM-842
blocks:
- OSYM-845
areas:
- debugging
- desktop
- tauri
parent: null
---

## Summary

Launch the selected issue workspace in Zed from the desktop app with accurate harness and attachment readiness, using the shared debug resolver.

## Scope

### In scope

- Use existing gateway/control APIs to resolve verified workspace, harness/profile, restoration availability and control state.
- Launch zed -n with an argv path and present the instruction to start the configured OpenSymphony Debug agent.
- Handle missing Zed, invalid workspace, lost owner/context, busy control and unsupported attachment capability with actionable UI states.
- Keep native Codex resume/--app routes available where advertised; UI consumes capabilities instead of a hard-coded OpenHands eligibility check.

### Out of scope

- Auto-starting editor threads without a documented API, embedding runtime clients in Tauri, and new debug manifests.

## Deliverables

- Capability-driven debug action, safe workspace launch plumbing and recovery UI.

## Acceptance Criteria

- [ ] Issues from two ACP profiles can launch the same IDE flow, displaying the correct bound harness/profile and control readiness.
- [ ] The launched path is the exact verified issue workspace, never a repository root, parent directory or runtime store.
- [ ] Native debug alternatives remain explicit and available; unavailable features are not presented as working actions.
- [ ] Failure states provide recovery steps without starting a second harness or changing scheduler state directly.

## Test Plan

- Test shared resolver payloads, path quoting/argv handling and UI capability states.
- Perform desktop-to-Zed launch smoke tests for two profiles; use shared fixtures for frontend tests.

## Context

- docs/specs/opensymphony-acp-debugging-spec.md: desktop and command surface.
- Shared API client/debug action surfaces and Tauri shell; OSYM-840 resolver and OSYM-842 guidance.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

The app opens the workspace; the user starts the static external agent. Acquiring runtime control belongs to the orchestrator handoff.
