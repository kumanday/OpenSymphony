---
name: custom-codereview-guide
description: |
  Repository-specific code review guidance for OpenSymphony.
  Update this file so automated PR review focuses on the right risks.
triggers:
  - /codereview
---

# Custom Code Review Guide

Automated PR review reads this guidance: the OpenHands PR Review plugin loads
it via the `/codereview` trigger, and Codex code review reaches it through the
`## Review guidelines` section in `AGENTS.md`.

**This is a durable, shared document.** Never add PR-specific or
ticket-specific content here — no "already resolved, do not re-flag" lists, no
per-PR evidence dumps, no review context for a single change. Respond to
review feedback in the PR's review threads instead. Only add guidance that
applies to all future reviews.

## Default Priorities

- Prioritize correctness, regressions, security risks, and missing tests ahead of style-only feedback.
- Treat behavior changes as incomplete unless the PR includes concrete verification or evidence.
- Call out risky data migrations, auth changes, concurrency hazards, and production operability regressions explicitly.

## OpenSymphony-Specific Review Focus

- **Orchestration authority**: the Rust orchestrator is the sole authority
  over scheduling state. Flag any background task that mutates scheduling
  state outside orchestrator-owned commands or messages.
- **Async/concurrency safety**: prefer actor ownership over shared locks. Flag
  new `Arc<Mutex<...>>` spread through the daemon, blocking operations inside
  async contexts, and missing cancellation handling.
- **Workspace safety**: workspace paths must remain inside the configured
  workspace root; every issue maps to exactly one sanitized workspace key; the
  agent runtime must execute with `cwd == issue_workspace_path`. Flag
  containment checks performed only after canonicalization and any path
  handling that can escape the root.
- **Error handling**: explicit error enums with context-rich messages. Flag
  swallowed errors and silent fallbacks that hide operational failures.
- **WebSocket resilience**: the runtime client must wait for readiness,
  reconcile the REST event backlog, deduplicate by event ID, preserve
  timestamp order, and reconnect with bounded exponential backoff. Flag
  changes that skip reconcile-after-reconnect.
- **Forward compatibility**: serde types should tolerate unknown
  fields/variants where the wire contract allows. Flag breaking wire-format
  changes that lack migration notes.
- **Credential hygiene**: never persist raw OAuth tokens, refresh material, or
  resolved account identifiers in manifests, logs, workpads, or debug output.

## Validation Expectations

- Rust: `cargo check-system-duckdb` / `cargo test-system-duckdb` /
  `cargo clippy-system-duckdb` (or the `-dev` aliases using
  `DUCKDB_DOWNLOAD_LIB=1`). Plain `cargo build`/`cargo test` without the
  DuckDB feature flags is not the supported validation path.
- Frontend: `npm run type-check` and `npm run test` at the workspace root.
- Full `cargo test` also runs memory integration tests that require
  `OPENSYMPHONY_MEMORY_ADMIN_TOKEN`; targeted test commands are acceptable
  evidence when that token is unavailable.

## Headless Shared UI Evidence

For `packages/ui-core` shared-shell changes, DOM fixture tests that mount
`renderOpenSymphonyApp` with `MockGatewayTransport` are acceptable product
evidence when the PR also includes a browser-rendered proof image or explains why
the unattended environment could not capture one. Do not block solely because the
evidence is not a full desktop app video when the code path under review is the
shared DOM renderer and the relevant selectors, links, token labels, and
visibility rules are asserted end-to-end.

## Evidence Expectations

- Behavior changes should include test or reproduction output.
- UI changes should include screenshots or recordings.
- Performance-sensitive changes should include benchmark data or timing notes.
