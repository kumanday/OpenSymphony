---
id: OSYM-506
title: Expand error-path tests for Linear client and workspace hooks
type: feature
area: quality-ops
priority: P1
estimate: 3d
milestone: M5 Validation and local packaging
parent: OSYM-500
depends_on:
  - OSYM-301
  - OSYM-302
  - OSYM-501
blocks: []
project_context:
  - AGENTS.md
  - docs/testing-and-operations.md
repo_paths:
  - crates/opensymphony-linear/
  - crates/opensymphony-workspace/
  - crates/opensymphony-testkit/
definition_of_ready:
  - Fake OpenHands server (OSYM-501) is in place for reuse patterns
  - Linear error taxonomy and workspace hook timeout policy are documented
---

# OSYM-506: Expand error-path tests for Linear client and workspace hooks

## Summary
Raise the test depth on the two highest-risk failure surfaces that currently have thin coverage: the Linear GraphQL client's error categorization and retry behavior, and the workspace hook timeout and termination pipeline.

## Scope
- Add unit and integration tests for `opensymphony-linear` covering request timeouts, 429 with `Retry-After`, 5xx retry eligibility, and partial GraphQL error payloads
- Verify every branch of `TrackerErrorCategory` mapping in `crates/opensymphony-linear/src/error.rs`
- Add workspace hook tests in `opensymphony-workspace` covering timeout escalation (SIGTERM then SIGKILL on Unix, `taskkill` equivalent on Windows), non-zero exit handling, and stdout/stderr capture limits
- Factor shared fakes/builders into `opensymphony-testkit` where useful

## Out of scope
- Changing the Linear client or workspace manager behavior
- Introducing a new mocking framework
- Live tests against `api.linear.app`

## Deliverables
- New tests under `crates/opensymphony-linear/tests/` and `crates/opensymphony-workspace/tests/`
- Reusable fakes/fixtures in `opensymphony-testkit` for HTTP error scenarios and controllable hook binaries
- Short note in `docs/testing-and-operations.md` describing how to author new error-path cases

## Acceptance criteria
- Every variant of `TrackerErrorCategory` is exercised by at least one test
- Hook timeout escalation is verified end-to-end on the host platform without flake in repeated runs
- Coverage of `opensymphony-linear` and `opensymphony-workspace` error modules measurably increases (see OSYM-508)

## Test plan
- New tests pass locally and in CI on Linux and macOS runners
- Repeat runs (`cargo test -p opensymphony-workspace --test workspace_manager -- --test-threads=1`) show no flake over at least 20 iterations
- Manual review to confirm no test depends on wall-clock sleeps longer than strictly necessary
