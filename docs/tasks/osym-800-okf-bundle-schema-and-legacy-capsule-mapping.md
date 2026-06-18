---
id: OSYM-800
title: OKF Bundle Schema And Legacy Capsule Mapping
milestone: "M10.5: OKF Memory Bundle Foundation"
priority: 2
estimate: 5
blockedBy: []
blocks: ["OSYM-801", "OSYM-802", "OSYM-803", "OSYM-820"]
areas:
  - memory
  - okf
parent: null
---

## Summary

Define the OpenSymphony OKF bundle schema and map the current memory capsule, milestone, project, area, repository, and topic-doc concepts onto it.

## Scope

### In scope

- Define internal structs for OKF concepts, bundle paths, reserved files, frontmatter fields, OpenSymphony extension metadata, links, citations, and visibility.
- Map existing issue capsule and generated memory node fields into OKF-required `type` plus recommended metadata.
- Preserve unknown frontmatter and legacy top-level fields during read and write.
- Document the logical bundle layout and migration strategy in the memory docs.

### Out of scope

- Moving the local durable memory store to the final OKF layout.
- Implementing graph rendering or hosted memory APIs.

## Deliverables

- OKF concept and bundle schema types in the memory implementation boundary.
- Legacy capsule to OKF mapping helpers.
- Documentation updates to `docs/memory.md` referencing `docs/okf-memory-spec.md`.

## Acceptance Criteria

- [ ] Existing memory capsules can be parsed into OKF concept records without losing legacy metadata.
- [ ] New OKF concept records require non-empty `type` and contained bundle-relative paths.
- [ ] Unknown fields round-trip through the parser and writer.
- [ ] The memory docs explain the chosen bundle layout and legacy compatibility model.

## Test Plan

- Add fixture tests for existing issue capsules, milestone nodes, and topic docs.
- Run `cargo test --test memory` or the focused memory crate tests covering concept parsing and path containment.
- Run `cargo fmt --check`.

## Context

- Read `docs/okf-memory-spec.md` before implementation.
- Inspect `docs/memory.md` and current memory capsule generation paths.
- Existing M6.5 memory-server work is a prerequisite foundation, but this task package tracks only the new OKF wave.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Repository remains a memory facet, not the top-level taxonomy.
