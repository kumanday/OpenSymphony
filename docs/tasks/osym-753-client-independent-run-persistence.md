---
id: OSYM-753
title: Client-Independent Run Persistence
milestone: "M11: Hosted Alpha"
priority: 1
estimate: 8
blockedBy: ["OSYM-704", "OSYM-750", "OSYM-752"]
blocks: ["OSYM-754", "OSYM-771"]
parent: null
---

## Summary

Ensure hosted runs, event journals, workspaces, and reconnect behavior continue correctly without connected clients.

## Scope

### In scope

- Persist run lifecycle state independent of active streams.
- Keep event journal recording while clients disconnect.
- Apply workspace cleanup and retention policy after terminal states.
- Support long disconnects followed by cursor replay or snapshot recovery.
- Verify a second permitted user can observe the same project and run state.

### Out of scope

- Full admin console.
- Model routing.
- Billing quotas.

## Deliverables

- Client disconnect tests.
- Long-running run tests.
- Reconnect and replay recovery path.
- Multi-user observation tests.

## Acceptance Criteria

- [ ] A hosted run continues after all clients disconnect.
- [ ] Reconnected clients recover committed event history and current state.
- [ ] Permitted users can observe shared project/run state and unauthorized users are blocked.

## Test Plan

- Run hosted disconnect/reconnect integration tests.
- Run long-running fake harness tests with event replay after client absence.
- Run multi-user permission tests.

## Context

- Source sections: `PRODUCT.md` hosted mode, `docs/hosted-client-PRD.md` hosted acceptance criteria, and `docs/host-client-implementation_plan.md` P8.7.
- This is the defining hosted-mode behavior for teams and disconnected users.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Client disconnects should affect UI state only; server execution should continue.
