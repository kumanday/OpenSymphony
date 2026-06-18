---
id: OSYM-801
title: OKF Writer, Lint, And Migration Fixtures
milestone: "M10.5: OKF Memory Bundle Foundation"
priority: 2
estimate: 8
blockedBy: ["OSYM-800"]
blocks: ["OSYM-802", "OSYM-803", "OSYM-804"]
areas:
  - memory
  - okf
  - cli
parent: null
---

## Summary

Add OKF-aware writing, linting, and migration fixtures so OpenSymphony can produce conformant bundles while reporting legacy gaps clearly.

## Scope

### In scope

- Add OKF write support for concept Markdown with YAML frontmatter, Markdown links, reserved `index.md`, and reserved `log.md`.
- Implement `opensymphony memory lint --okf` diagnostics for errors, warnings, and info messages described in the spec.
- Add migration fixtures for legacy issue capsules, date-grouped logs, citations, wiki-link compatibility, and broken Markdown links.
- Keep warning-level issues nonfatal unless they could leak private data or break containment.

### Out of scope

- Public/private export packaging.
- Graph DTO extraction.

## Deliverables

- OKF writer and linter code paths.
- CLI lint flag and diagnostic rendering.
- Fixture corpus for legacy and OKF-conformant documents.

## Acceptance Criteria

- [ ] `opensymphony memory lint --okf` reports missing frontmatter, invalid YAML, missing `type`, invalid reserved files, and private export leaks as errors.
- [ ] Warnings cover missing recommended fields, broken links, wiki-only links, missing generated indexes, and missing citations.
- [ ] Generated `log.md` output uses ISO date headings with newest entries first.
- [ ] Legacy fixtures can be migrated or enriched without dropping unknown metadata.

## Test Plan

- Run focused OKF lint fixture tests.
- Run `opensymphony memory lint --okf` against a generated fixture bundle.
- Run `cargo test --test memory`.

## Context

- Builds on OSYM-800.
- Source spec: `docs/okf-memory-spec.md` sections 4, 6, 10, and 11.
- Preserve private memory boundaries from `docs/memory.md`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use actionable diagnostics; agents should be able to fix the reported file directly.
