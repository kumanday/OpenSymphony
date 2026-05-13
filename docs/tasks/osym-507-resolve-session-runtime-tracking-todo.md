---
id: OSYM-507
title: Resolve runtime tracking TODO in OpenHands session runner
type: chore
area: openhands
priority: P2
estimate: 1d
milestone: M5 Validation and local packaging
parent: OSYM-500
depends_on:
  - OSYM-204
blocks: []
project_context:
  - AGENTS.md
  - docs/websocket-runtime.md
repo_paths:
  - crates/opensymphony-openhands/src/session.rs
definition_of_ready:
  - Behavior expected at the TODO site is captured in a short design note or comment
---

# OSYM-507: Resolve runtime tracking TODO in OpenHands session runner

## Summary
Close out the lingering runtime-tracking TODO in the OpenHands session runner so the module has a clean clippy/warning surface and the intended behavior is either implemented or explicitly deferred with a tracked reference.

## Scope
- Investigate the TODO at `crates/opensymphony-openhands/src/session.rs:2815` and document the intended runtime-tracking behavior
- Either implement the missing logic or replace the TODO with a short comment that links to a follow-on issue and narrows the condition to the actual gap
- Ensure `clippy::todo` (already a workspace warning) has no remaining occurrences after the change

## Out of scope
- Broader refactors of the session runner (covered by OSYM-505)
- Changes to the WebSocket event contract

## Deliverables
- Updated `session.rs` with the TODO resolved or replaced by a precise, linked comment
- Test or assertion that exercises the newly defined behavior when logic is added

## Acceptance criteria
- `rg "TODO|FIXME" crates/opensymphony-openhands/src` returns no hits, or every remaining hit is a linked reference to a tracked issue with a deadline
- `cargo clippy -p opensymphony-openhands -- -D warnings` passes with no allow attributes introduced
- Any new behavior is covered by at least one unit or integration test

## Test plan
- `cargo test -p opensymphony-openhands`
- If new behavior is implemented, live suite run (`OPENSYMPHONY_LIVE_OPENHANDS=1 cargo test --test live_local_suite -- --ignored --nocapture`)
