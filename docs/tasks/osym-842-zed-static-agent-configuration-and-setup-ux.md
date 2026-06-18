---
id: OSYM-842
title: Zed Static Agent Configuration And Setup UX
milestone: "M12.5: ACP Debugging And IDE Attach"
priority: 3
estimate: 3
blockedBy: ["OSYM-841"]
blocks: ["OSYM-843", "OSYM-844"]
areas:
  - debugging
  - docs
  - acp
parent: null
---

## Summary

Document and surface the one-time Zed external-agent configuration needed to start OpenSymphony Debug through ACP.

## Scope

### In scope

- Document the static Zed `agent_servers.opensymphony-debug` configuration.
- Add CLI or app guidance for missing configuration and invalid workspace shape.
- Provide copyable setup text without writing per-issue Zed configuration.
- Include troubleshooting for missing manifests, invalid cwd, and existing OpenHands server store mismatch.

### Out of scope

- Automatically starting a Zed agent thread from the CLI.
- Writing per-conversation Zed `agent_servers` entries.

## Deliverables

- Zed setup documentation.
- Operator-facing setup and recovery messages.
- Tests for guidance text where CLI output is covered.

## Acceptance Criteria

- [ ] A single static Zed external-agent config can start `opensymphony debug --acp-stdio`.
- [ ] Documentation shows how to open an issue workspace and start the OpenSymphony Debug agent.
- [ ] Error guidance tells users to open the exact issue workspace root.
- [ ] No per-issue Zed agent configuration is created.

## Test Plan

- Run docs link checks or markdown lint if available.
- Run CLI output tests for setup and failure guidance where implemented.
- Manually verify the documented Zed JSON snippet is syntactically valid.

## Context

- Builds on OSYM-841.
- Read `docs/opensymphony-acp-debugging-spec.md` Zed integration and desired UX sections.
- The expected static command is `opensymphony debug --acp-stdio`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

At MVP scope, the operator starts the external agent from Zed.
