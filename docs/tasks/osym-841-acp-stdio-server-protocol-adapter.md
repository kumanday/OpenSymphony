---
id: OSYM-841
title: ACP Stdio Server Protocol Adapter
milestone: "M12.5: ACP Debugging And IDE Attach"
priority: 2
estimate: 13
blockedBy: ["OSYM-840"]
blocks: ["OSYM-842", "OSYM-845"]
areas:
  - debugging
  - acp
  - cli
parent: null
---

## Summary

Add `opensymphony debug --acp-stdio` as a noninteractive ACP JSON-RPC server that attaches Zed or another ACP client to an existing issue conversation.

## Scope

### In scope

- Implement ACP stdio server mode under the existing `debug` command family.
- Support initialize, `session/new`, `session/prompt`, and `session/close`.
- Treat `session/new.params.cwd` as the authoritative workspace selection input.
- Enforce one active ACP debug session per spawned process.
- Ensure stdout contains only protocol messages in ACP mode.

### Out of scope

- ACP `session/list`, `session/load`, and `session/resume`.
- Zed launch UI.

## Deliverables

- ACP stdio protocol adapter.
- Strict cwd validation and actionable ACP errors.
- Event mapping from normalized OpenHands runtime events to ACP updates.

## Acceptance Criteria

- [ ] `opensymphony debug --acp-stdio` starts without requiring an issue key.
- [ ] `session/new` rejects parent workspace roots, nested paths, target repo roots, and OpenHands conversation store paths.
- [ ] `session/prompt` sends the message, runs the existing OpenHands conversation, and streams useful updates.
- [ ] `session/close` detaches without deleting workspaces, manifests, memory, or OpenHands conversations.

## Test Plan

- Add ACP stdio unit tests with a minimal JSON-RPC harness.
- Add fixture tests for valid cwd and invalid cwd variants.
- Run focused debug-session and OpenHands runtime tests.

## Context

- Builds on OSYM-840.
- Read `docs/opensymphony-acp-debugging-spec.md` command surface, ACP method behavior, event mapping, concurrency, and failure sections.
- Keep protocol output off human-readable stdout.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The ACP session id is an attachment id, not a raw OpenHands conversation id.
