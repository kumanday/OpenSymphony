---
id: OSYM-844
title: Default Debug UX Transition And CLI Compatibility
milestone: "M12.5: ACP Debugging And IDE Attach"
priority: 3
estimate: 5
blockedBy: ["OSYM-840", "OSYM-842", "OSYM-843"]
blocks: ["OSYM-845"]
areas:
  - debugging
  - cli
  - desktop
parent: null
---

## Summary

Transition `opensymphony debug <issue-key>` toward the IDE-oriented debug workflow while preserving terminal debug compatibility through `--cli`.

## Scope

### In scope

- Make the default debug command resolve or prepare the preferred IDE debug experience after ACP and Tauri launch paths are stable.
- Preserve terminal interactive debugging through `opensymphony debug <issue-key> --cli`.
- Print concise fallback instructions when Zed or the desktop integration is unavailable.
- Update CLI help, operations docs, and debugging docs.

### Out of scope

- Removing terminal debug support.
- Hosted browser IDE integration.

## Deliverables

- Updated debug command behavior and flags.
- CLI help and operations docs.
- Regression tests for `--cli`, `--acp-stdio`, and default debug paths.

## Acceptance Criteria

- [ ] `opensymphony debug <issue-key> --cli` preserves the terminal workflow.
- [ ] `opensymphony debug --acp-stdio` remains noninteractive and does not require an issue key.
- [ ] `opensymphony debug <issue-key>` produces the IDE-oriented path or clear fallback guidance.
- [ ] Documentation explains the mode split without encouraging unsupported OpenHands protocols.

## Test Plan

- Run CLI debug regression tests.
- Run docs checks for debug command references.
- Manually verify help output for default, `--cli`, and `--acp-stdio` paths.

## Context

- Builds on OSYM-840, OSYM-842, and OSYM-843.
- Read `docs/opensymphony-acp-debugging-spec.md` command surface and default debug UX transition sections.
- Respect AGENTS.md: OpenSymphony targets SDK agent-server REST plus WebSocket, not web-app Socket.IO.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This task should happen after the ACP and Zed launch paths are demonstrably stable.
