---
id: OSYM-782
title: TUI Codex Token Usage Accounting
milestone: "M10.4: Desktop Live Operations And Model Polish"
priority: 3
estimate: 3
blockedBy: []
blocks: []
areas:
  - tui
  - codex
  - runtime
parent: null
---

## Summary

Fix token usage reporting for Codex-backed runs in the TUI.

## Scope

### In scope

- Inspect Codex `thread/tokenUsage/updated` events and the current normalized event path.
- Map Codex token usage payloads into OpenSymphony input, output, cache-read, and total token counters where the payload provides those fields.
- Ensure TUI issue detail and top status totals update as Codex token usage events arrive.
- Preserve current OpenHands token behavior.
- Add a focused regression test or fake Codex event fixture for token usage updates.

### Out of scope

- Cost estimation.
- Provider-specific billing policy.
- Reworking token accounting for non-Codex harnesses beyond avoiding regressions.

## Deliverables

- Codex token usage normalization or snapshot update fix.
- TUI display update if the current counters are reading the wrong fields.
- Regression coverage for a Codex token usage update.

## Acceptance Criteria

- [ ] A Codex run that emits `thread/tokenUsage/updated` updates TUI token counters without restarting the TUI.
- [ ] Input, output, cache-read, and total counters are populated when present in the Codex event payload.
- [ ] Missing token fields degrade to existing zero/unknown behavior without panics.
- [ ] OpenHands token counters continue to pass existing tests.

## Test Plan

- Run the focused Codex normalization/runtime tests.
- Run affected TUI snapshot/control-plane tests.
- Manually verify with a local Codex-backed run that the TUI token totals change after token usage events.

## Context

- Live Codex-backed TUI runs show `thread/tokenUsage/updated` events, but the visible token counters remain incorrect or static.
- The fix should follow the existing normalized runtime event and control-plane snapshot path rather than adding a TUI-only parser.

## Definition of Ready

- [ ] Example Codex token usage payload is captured or fixture-backed.
- [ ] Current token counter source in TUI and control-plane snapshots is identified.
- [ ] A coding agent can start from this task without additional planning context.
