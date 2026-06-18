---
id: OSYM-821
title: Graph Extraction, Metrics, And Community Pipeline
milestone: "M11.5: LLM Wiki Graph View"
priority: 2
estimate: 8
blockedBy: ["OSYM-820"]
blocks: ["OSYM-823", "OSYM-824", "OSYM-825"]
areas:
  - memory
  - graph-view
parent: null
---

## Summary

Extract graph nodes, edges, metrics, and communities from OKF concepts so clients receive a meaningful navigable memory graph.

## Scope

### In scope

- Extract bundle, directory, concept, tag, resource, citation, source-ref, and community nodes.
- Extract contains, Markdown link, external link, cites, tagged-with, describes-resource, scoped-to, source-supported-by, and same-resource edges.
- Compute degree, centrality or bridge score, orphan count, broken-link count, stale concept count, warning count, and community IDs.
- Evaluate and select graph metric and community detection libraries instead of hand-rolling physics or clustering.

### Out of scope

- Client rendering.
- Editing graph links or OKF concepts.

## Deliverables

- Graph extraction pipeline from OKF-derived catalog records.
- Metric and community computation with deterministic fixture behavior.
- Dependency evaluation note if new graph libraries are added.

## Acceptance Criteria

- [ ] OKF fixture concepts produce deterministic graph snapshots.
- [ ] Broken Markdown links appear as unresolved targets or warnings without failing extraction.
- [ ] Community labels derive from dominant concept types, tags, areas, or directories.
- [ ] Tags, citations, and source refs can be included or excluded from community detection.

## Test Plan

- Add OKF fixture graph extraction tests.
- Add community stability tests for a fixed fixture corpus.
- Run memory and gateway tests that consume graph snapshots.

## Context

- Builds on OSYM-820.
- Read `docs/llm-wiki-graph-view-spec.md` sections 6 and 9.
- Graph relationships remain derived; memory documents are the source of truth.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Plain Markdown links should not be reinterpreted as strong semantic relations unless typed metadata supports that claim.
