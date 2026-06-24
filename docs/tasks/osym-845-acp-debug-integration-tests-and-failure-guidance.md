---
id: OSYM-845
title: ACP Debug Integration Tests And Failure Guidance
milestone: "M12.5: ACP Debugging And IDE Attach"
priority: 2
estimate: 8
blockedBy: ["OSYM-840", "OSYM-841", "OSYM-843", "OSYM-844"]
blocks: []
areas:
  - debugging
  - acp
  - testing
parent: null
---

## Summary

Add end-to-end ACP debug coverage and polish failure guidance for invalid workspaces, missing manifests, missing conversations, active turns, and store mismatches.

## Scope

### In scope

- Add a minimal ACP stdio client harness around the fake OpenHands server.
- Test initialize, `session/new`, `session/prompt`, `session/close`, and stream detach behavior.
- Cover invalid cwd variants, missing conversation manifest, invalid conversation id, missing OpenHands conversation, already-running turn, and existing server with different store.
- Verify ACP close leaves durable issue workspaces, manifests, memory, and OpenHands conversations intact.

### Out of scope

- Zed UI automation.
- Multi-session ACP multiplexing.

## Deliverables

- ACP debug integration test harness.
- Failure-mode tests and operator guidance assertions.
- Documentation updates for troubleshooting.

## Acceptance Criteria

- [ ] A fake-server integration test attaches to a fixture issue workspace and runs a prompt through ACP.
- [ ] Invalid cwd errors identify the expected exact issue workspace shape.
- [ ] Missing manifest and invalid conversation id errors name the relevant file or field.
- [ ] `session/close` is verified as detach-only behavior.

## Test Plan

- Run the ACP debug integration test harness.
- Run debug-session unit and CLI regression tests.
- Run fake OpenHands server contract tests touched by debug attachment.

## Context

- Builds on OSYM-840, OSYM-841, OSYM-843, and OSYM-844.
- Read `docs/specs/opensymphony-acp-debugging-spec.md` failure behavior, test plan, and acceptance criteria.
- Keep legacy flat, active, and archived OpenHands stores supported.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The test harness should not depend on Zed being installed.
