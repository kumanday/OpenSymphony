---
id: OSYM-504
title: Add supply-chain and security audits to CI
type: chore
area: quality-ops
priority: P0
estimate: 1d
milestone: M5 Validation and local packaging
parent: OSYM-500
depends_on:
  - OSYM-503
blocks:
  - OSYM-602
project_context:
  - AGENTS.md
  - README.md
  - docs/testing-and-operations.md
repo_paths:
  - .github/workflows/
  - Cargo.toml
  - Cargo.lock
definition_of_ready:
  - Baseline CI (fmt, clippy, test) is green on main
  - Deny/audit policy scope is agreed
---

# OSYM-504: Add supply-chain and security audits to CI

## Summary
Extend the existing CI pipeline with dependency vulnerability scanning and license/source policy enforcement so regressions in the dependency tree are caught automatically on every pull request.

## Scope
- Add `cargo audit` (RustSec advisory database) as a required CI step
- Add `cargo deny check` with a workspace-level `deny.toml` covering advisories, bans, licenses, and sources
- Pin toolchain version used for the audit jobs to match `rust-toolchain.toml`
- Document how to update advisory ignores and license allowlists in `docs/testing-and-operations.md`

## Out of scope
- Replacing existing clippy/fmt/test jobs
- Signing releases or SBOM generation
- Automated dependency upgrades (dependabot/renovate configuration)

## Deliverables
- `deny.toml` at the workspace root
- New CI job(s) in `.github/workflows/ci.yml` (or a dedicated `audit.yml`) running `cargo audit` and `cargo deny check`
- Short operations note covering triage of advisory and license failures

## Acceptance criteria
- A pull request that introduces a dependency with a known RustSec advisory fails CI
- A pull request that introduces a dependency with a non-allowlisted license fails CI
- `deny.toml` and any ignore entries are documented and time-bounded where applicable

## Test plan
- Smoke-test the new jobs on a throwaway branch by temporarily adding a known-flagged dependency
- Verify that unrelated PRs complete within acceptable additional CI time
- Confirm that cached advisory-db lookups do not break scheduled runs
