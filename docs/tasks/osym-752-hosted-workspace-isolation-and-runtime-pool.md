---
id: OSYM-752
title: Hosted Workspace Isolation And Runtime Pool
milestone: "M6: Hosted Alpha"
priority: 1
estimate: 13
blockedBy: ["OSYM-723", "OSYM-725", "OSYM-750", "OSYM-751"]
blocks: ["OSYM-753"]
parent: null
---

## Summary

Add hosted workspace isolation and a server-owned OpenHands runtime pool for tenant-scoped agent execution.

## Scope

### In scope

- Select hosted isolation layer: containers, VMs, or managed sandbox.
- Define hosted workspace lifecycle, network policy, filesystem policy, cleanup, and retention.
- Integrate workspace manager with logical workspace IDs.
- Run OpenHands agent-server instances under platform control.
- Route conversations to isolated workspaces.
- Manage runtime health checks, capacity, resource limits, and server-owned event attachment.

### Out of scope

- Codex app-server runtime pool.
- Hosted admin UI.
- Cross-harness routing policy.

## Deliverables

- Workspace isolation implementation.
- Isolation test plan.
- Hosted harness runtime manager.
- Runtime pool tests.

## Acceptance Criteria

- [ ] Hosted workspaces are isolated by tenant, project, and run.
- [ ] Hosted OpenHands runtime sessions attach server-side and stream normalized events into the journal.
- [ ] Resource limits and cleanup policies are enforced or explicitly documented for alpha.

## Test Plan

- Run workspace isolation tests for filesystem and tenant boundaries.
- Run hosted OpenHands pool tests with fake and local runtime fixtures.
- Run resource limit and cleanup tests for completed, failed, and cancelled runs.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.10 and `docs/host-client-architecture.md` 9.
- Hosted execution can run arbitrary code, so workspace isolation is a core alpha requirement.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use logical workspace IDs in gateway responses so clients remain independent of physical hosted paths.
