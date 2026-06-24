---
id: OSYM-820
title: Memory Graph DTOs And Gateway Endpoints
milestone: "M11.5: LLM Wiki Graph View"
priority: 2
estimate: 8
blockedBy: ["OSYM-800", "OSYM-802"]
blocks: ["OSYM-821", "OSYM-822", "OSYM-825"]
areas:
  - gateway
  - memory
  - graph-view
parent: null
---

## Summary

Define and expose versioned memory graph DTOs for bundles, graph snapshots, concept details, communities, search, and update events.

## Scope

### In scope

- Add DTOs for bundle list, graph snapshot, concept detail, communities, search, graph nodes, graph edges, and schema versions.
- Expose gateway or memory-server endpoints for `GET /api/v1/memory/bundles`, bundle graph snapshots, concept detail, communities, and search.
- Add `memory_graph_updated` event stream contract.
- Normalize visibility and local-path redaction at the data boundary.

### Out of scope

- Three.js rendering.
- Graph editing or memory mutation from the client.

## Deliverables

- Shared DTO models and serialization tests.
- Gateway or memory-server read endpoints.
- Contract documentation for graph endpoints and update events.

## Acceptance Criteria

- [ ] Clients can request accessible bundles and receive schema-versioned DTOs.
- [ ] Concept detail separates primary, OpenSymphony, unknown, links, citations, and source refs.
- [ ] Graph DTOs include required node and edge kinds from the spec.
- [ ] Unauthorized private concepts and local paths are filtered before DTOs leave the server boundary.

## Test Plan

- Add DTO serialization and compatibility tests.
- Add endpoint contract tests using OKF fixtures.
- Run gateway/control-plane tests affected by the new routes.

## Context

- Read `docs/specs/llm-wiki-graph-view-spec.md` sections 6 and 7.
- Builds on OSYM-800 and OSYM-802.
- The client should not parse private files directly during normal operation.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use stable DTOs that work for loopback HTTP, hosted HTTPS, and Tauri adapters.
