---
id: OSYM-715
title: Desktop Connection Profiles And Daemon Management
milestone: "M2: Shared Client And Desktop Alpha"
priority: 2
estimate: 5
blockedBy: ["OSYM-702", "OSYM-711", "OSYM-714"]
blocks: ["OSYM-716", "OSYM-717"]
parent: null
---

## Summary

Implement desktop connection profiles and local daemon discovery/supervision for local, external, and hosted OpenSymphony modes.

## Scope

### In scope

- Add local daemon, supervised local daemon, embedded/direct host, local native IPC, external gateway, and hosted gateway profiles.
- Store connection profile settings locally.
- Probe default loopback gateway and validate `/healthz` plus `/api/v1/capabilities`.
- Allow manual gateway URL override.
- Start and monitor a local daemon when configured, tracking process ownership.
- Surface daemon logs and startup errors.

### Out of scope

- Hosted login flow.
- Credential secrets.
- Final native stream optimization.

## Deliverables

- Connection profile UI and storage.
- Discovery command and UI.
- Optional local daemon supervisor.
- Process ownership tests.

## Acceptance Criteria

- [ ] Desktop users can switch between local daemon, external gateway, and hosted gateway profiles.
- [ ] The app starts only configured supervised daemons and stops only processes it owns.
- [ ] Capability discovery drives which actions and streams are enabled.

## Test Plan

- Run desktop profile unit tests.
- Run local daemon discovery tests against healthy, missing, and incompatible gateway fixtures.
- Run process ownership tests with fake daemon commands.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.2.2 and `docs/host-client-architecture.md` 3.1 through 3.3.
- Local desktop operation can use a different physical transport from hosted operation while preserving the same frontend contract.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Manual gateway override is important for external local server mode and protocol testing.
