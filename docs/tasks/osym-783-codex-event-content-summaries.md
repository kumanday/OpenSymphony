---
id: OSYM-783
title: Codex Event Content Summaries
milestone: "M10.4: Desktop Live Operations And Model Polish"
priority: 3
estimate: 5
blockedBy: []
blocks: []
areas:
  - codex
  - tui
  - desktop
parent: null
---

## Summary

Extract useful human-readable summaries from Codex events so TUI and desktop activity views show content instead of only generic event kind identifiers.

## Scope

### In scope

- Inspect current Codex app-server notification payloads for message deltas, command execution output, diff updates, approvals, and completion events.
- Extract bounded, redacted summaries for high-value Codex event kinds such as `item/agentMessage/delta`, `item/commandExecution/outputDelta`, `turn/diff/updated`, and `thread/tokenUsage/updated`.
- Keep raw payload retention for diagnostics while rendering safer summaries to clients.
- Update normalized event summaries used by both TUI conversation activity and desktop inspector activity.
- Add fixture coverage for representative Codex event payloads.

### Out of scope

- Full transcript persistence.
- Streaming every token delta verbatim into Linear or memory.
- Rendering a rich terminal emulator in the activity list.

## Deliverables

- Codex event summary extraction helpers.
- Redaction/bounding behavior for message and command-output previews.
- Shared activity view improvement for TUI and desktop.
- Fixture tests for representative Codex events.

## Acceptance Criteria

- [ ] TUI conversation activity shows useful Codex message or command-output previews where available, not only `Codex event: <kind>`.
- [ ] Desktop activity view receives the same improved summaries through the existing gateway event surfaces.
- [ ] Summaries are bounded and do not expose raw secrets beyond the existing runtime payload policy.
- [ ] Unknown or unsupported Codex event kinds still render a stable generic fallback.

## Test Plan

- Run Codex event normalization tests with representative app-server payload fixtures.
- Run affected TUI/activity rendering tests.
- Manually verify a local Codex-backed run shows more informative activity entries in both TUI and desktop.

## Context

- Current live Codex conversation activity repeatedly renders generic lines such as `Codex event: item/agentMessage/delta` and `Codex event: item/commandExecution/outputDelta`.
- The improvement should happen in the shared normalization path so TUI and desktop benefit together.

## Definition of Ready

- [ ] Representative Codex event payloads are captured or fixture-backed.
- [ ] Existing summary generation and redaction points are identified.
- [ ] A coding agent can start from this task without additional planning context.
