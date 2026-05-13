---
id: OSYM-508
title: Wire cargo-llvm-cov coverage reporting and regression floor into CI
type: chore
area: quality-ops
priority: P1
estimate: 2d
milestone: M5 Validation and local packaging
parent: OSYM-500
depends_on:
  - OSYM-501
  - OSYM-506
blocks:
  - OSYM-602
project_context:
  - AGENTS.md
  - docs/testing-and-operations.md
repo_paths:
  - .github/workflows/
  - scripts/
definition_of_ready:
  - Error-path tests from OSYM-506 are merged so the measured baseline is representative
  - Minimum coverage threshold is agreed with maintainers
---

# OSYM-508: Wire cargo-llvm-cov coverage reporting and regression floor into CI

## Summary
Add line and region coverage reporting via `cargo-llvm-cov` so coverage regressions are visible on each pull request, and enforce a modest floor to prevent silent drops.

## Scope
- Add a coverage job to CI running `cargo llvm-cov --workspace --lcov --output-path lcov.info`
- Upload the `lcov.info` artifact and, if available, publish a coverage summary comment on pull requests
- Enforce a minimum per-workspace line-coverage threshold (initial suggestion: 60 percent) configurable via repo variable
- Document how to reproduce coverage locally in `docs/testing-and-operations.md`

## Out of scope
- Branch or mutation testing
- Per-crate coverage thresholds beyond the workspace floor
- Third-party coverage-hosting integrations that require paid accounts

## Deliverables
- New CI job producing `lcov.info` and a printable summary
- Optional helper script under `scripts/` for running coverage locally with the same flags as CI
- Updated operations documentation covering local reproduction and threshold adjustment

## Acceptance criteria
- A pull request that drops workspace line coverage below the configured floor fails CI with a clear message
- Coverage artifacts are retrievable from a completed CI run
- Local reproduction command produces results within 10 percent of the CI-reported number

## Test plan
- Dry run on a feature branch to establish the initial floor
- Intentionally remove a covered test on a throwaway branch to confirm the floor triggers a failure
- Confirm that the coverage job does not materially slow the required PR checks (target: under 50 percent overhead vs. the plain test job)
