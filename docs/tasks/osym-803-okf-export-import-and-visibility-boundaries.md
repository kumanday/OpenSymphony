---
id: OSYM-803
title: OKF Export, Import, And Visibility Boundaries
milestone: "M10.5: OKF Memory Bundle Foundation"
priority: 2
estimate: 8
blockedBy: ["OSYM-801", "OSYM-802"]
blocks: ["OSYM-804", "OSYM-825"]
areas:
  - memory
  - okf
  - security
parent: null
---

## Summary

Add OKF import and export flows with explicit public/private variants and redaction checks that prevent private material from leaking.

## Scope

### In scope

- Implement `opensymphony memory export-okf --visibility public` and private export with output path support.
- Implement `opensymphony memory import-okf path/to/bundle` with validation, unknown field preservation, and actionable diagnostics.
- Redact or exclude private capsules, private comments, private local paths, and private source snapshots from public exports.
- Add ZIP or directory export support according to existing CLI conventions.

### Out of scope

- Hosted tenant import UI.
- Editing OKF concepts after import.

## Deliverables

- Export and import CLI paths.
- Public/private export redaction tests.
- Import validation tests for unknown types, missing optional fields, broken links, and missing indexes.

## Acceptance Criteria

- [ ] Public export excludes private concepts and private source material.
- [ ] Private export can include local-private concepts without weakening source citation boundaries.
- [ ] Import tolerates unknown concept types and unknown frontmatter fields.
- [ ] Import reports malformed concepts with actionable file paths.

## Test Plan

- Run export/import fixture tests.
- Run public export redaction tests.
- Run `opensymphony memory lint --okf` on exported fixtures.

## Context

- Builds on OSYM-801 and OSYM-802.
- Read `docs/okf-memory-spec.md` sections 5, 6, 9, and 12.
- Public docs must not link directly to private bundle paths.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Treat redaction failures as errors, not warnings.
