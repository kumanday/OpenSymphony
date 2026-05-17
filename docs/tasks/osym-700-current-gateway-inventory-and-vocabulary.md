---
id: OSYM-700
title: Current Gateway Inventory And Vocabulary
milestone: "M6: Gateway And Stream Contract"
priority: 1
estimate: 3
blockedBy: []
blocks: ["OSYM-701", "OSYM-702", "OSYM-703"]
parent: null
---

## Summary

Inventory the current OpenSymphony control plane and define the public vocabulary that the rich clients, hosted mode, task graph, and planning workflow will share.

## Scope

### In scope

- Catalog current health, snapshot, event, CLI, Linear, and OpenHands runtime surfaces.
- Define `Project`, `Milestone`, `Issue`, `SubIssue`, `Run`, `Workspace`, `HarnessSession`, `TerminalSession`, `PlanningSession`, and `Artifact`.
- Define Linear-to-OpenSymphony ID mapping rules for local and hosted modes.
- Identify private orchestrator structs and fields that need DTO boundaries.

### Out of scope

- Gateway implementation.
- Frontend package setup.
- Hosted identity design.

## Deliverables

- Current API inventory document.
- Domain vocabulary and ID mapping notes.
- Public/private boundary checklist for the gateway work.

## Acceptance Criteria

- [ ] Existing control-plane endpoints and payloads are documented with repo paths and sample shapes.
- [ ] The Linear project/milestone/issue/sub-issue taxonomy is defined consistently with the PRD.
- [ ] Gateway DTO boundary risks are listed with owners for follow-up tasks.

## Test Plan

- Run `cargo test` to verify the inventory work did not change behavior.
- Review the inventory against `docs/host-client-implementation_plan.md` P0.1 and P0.2.

## Context

- Read `PRODUCT.md`, `docs/hosted-client-PRD.md`, `docs/host-client-architecture.md`, and `docs/host-client-implementation_plan.md`.
- Inspect `crates/opensymphony-control`, `crates/opensymphony-orchestrator`, `crates/opensymphony-linear`, and `crates/opensymphony-openhands`.
- Preserve AGENTS.md invariants: orchestrator-owned scheduling state, server-owned harness attachment, and UI separation.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This task sets the language used by every later task. Use Linear-native names for user-facing task graph concepts.
