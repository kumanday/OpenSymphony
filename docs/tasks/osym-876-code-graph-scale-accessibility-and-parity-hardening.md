---
id: OSYM-876
title: Code Graph Scale Accessibility And Parity Hardening
milestone: "M12.9: Code Graph View"
priority: 2
estimate: 5
blockedBy: ["OSYM-870", "OSYM-872", "OSYM-873", "OSYM-874", "OSYM-875"]
blocks: []
areas:
  - graph-view
  - accessibility
  - testing
parent: null
---

## Summary

Harden the Code Graph for edge-heavy repositories, accessibility parity, web/desktop rendering parity, and security-sensitive boundary regressions.

## Scope

### In scope

- Add edge-heavy fixture tiers and aggregated Atlas reference-scale fixtures.
- Verify truncation reporting and enforce no unaggregated full-repo Atlas render path.
- Add WebGL nonblank, canvas fallback, and visual regression checks for Code Graph states.
- Verify keyboard navigation, list fallbacks, screen-reader summaries, reduced motion, and non-color confidence/delta encodings.
- Verify HTTP and native-command parity on identical fixtures.
- Add path-redaction, hosted snippet denial, stale marker, and unsupported-file regression tests.
- Update user and developer docs for Code Graph operation and limitations.

### Out of scope

- Performance work outside the Code Graph and shared renderer paths.
- New graph layout libraries.
- Hosted automatic indexing policy changes.

## Deliverables

- Scale and edge-heavy fixtures.
- Web and desktop visual/accessibility regression coverage.
- Contract parity and security-boundary tests.
- Code Graph documentation updates.
- Final validation notes for the milestone.

## Acceptance Criteria

- [ ] Neighborhood, File, and aggregated Atlas fixtures stay within the performance budgets from the spec.
- [ ] Truncated responses report the truncation reason and counts.
- [ ] Code Graph renders nonblank through WebGL and canvas fallback in web and desktop test paths.
- [ ] Keyboard and list fallback flows are available for Atlas, File, Neighborhood, and Diff modes.
- [ ] Confidence, freshness, delta status, and diagnostics are not encoded by color alone.
- [ ] HTTP and native commands agree on fixture DTOs.
- [ ] No tested response leaks absolute paths, workspace roots, or hosted-forbidden snippets.

## Test Plan

- Run graph renderer fixture tests, visual regression checks, and accessibility checks.
- Run gateway and native command parity tests.
- Run focused security/redaction tests for code DTOs and snippets.
- Run `cargo fmt --check`.
- Run `git diff --check`.

## Context

- Read `docs/specs/code-graph-view-spec.md` sections 12, 13, 14, and 16.
- Inspect OSYM-826 graph scale and visual regression patterns.
- Inspect existing accessibility fallback behavior in the Knowledge Graph.
- This task is the final milestone hardening pass after feature slices are integrated.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep performance proof fixture-driven. Do not add a second renderer benchmark framework unless the existing checks cannot cover the risk.
