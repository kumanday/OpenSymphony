---
id: OSYM-509
title: Audit tracing spans and diagnostics for secret leakage
type: chore
area: quality-ops
priority: P0
estimate: 2d
milestone: M5 Validation and local packaging
parent: OSYM-500
depends_on:
  - OSYM-202
  - OSYM-203
  - OSYM-302
blocks: []
project_context:
  - AGENTS.md
  - README.md
  - docs/configuration.md
  - docs/deployment-modes.md
repo_paths:
  - crates/opensymphony-openhands/
  - crates/opensymphony-linear/
  - crates/opensymphony-cli/
  - crates/opensymphony-control/
definition_of_ready:
  - List of sensitive values (Linear API key, OpenHands session key, LLM API keys, WebSocket query-auth tokens, git credentials) is agreed
  - Diagnostics bundle contents for `doctor` are documented
---

# OSYM-509: Audit tracing spans and diagnostics for secret leakage

## Summary
Audit every `tracing` span, `Debug` implementation, diagnostic dump, and doctor output for accidental inclusion of API keys, session tokens, auth headers, or full URLs containing query-string credentials, and close any gaps found.

## Scope
- Review all `tracing::{trace,debug,info,warn,error}!` call sites in network-adjacent crates (`opensymphony-openhands`, `opensymphony-linear`, `opensymphony-control`, `opensymphony-cli`)
- Check `Debug` derives on config structs that carry secrets; replace with manual impls or `secrecy::Secret<T>`-style wrappers where needed
- Verify WebSocket query-auth mode never logs the full URL or connection string
- Verify Linear `Authorization` header is never included in request/response debug logs
- Audit conversation-fingerprint generation (`sha2` inputs) to confirm no raw API key is hashed
- Review `doctor`, `rehydrate`, and snapshot export paths for secret inclusion

## Out of scope
- Introducing a structured log redaction framework
- Rotating or changing the authentication mechanisms themselves

## Deliverables
- Audit checklist with findings recorded in the pull request description
- Code changes that redact or wrap identified secrets at their logging/serialization sites
- Unit tests that assert redacted rendering for secret-bearing types (for example, `format!("{:?}", cfg)` must not contain the API key)
- Short guidance in `docs/configuration.md` on how new secret-bearing fields should be declared and tested

## Acceptance criteria
- No `tracing` call or `Debug` output in the audited crates emits an API key, session key, bearer token, or query-auth token
- Every type that carries a secret has a test asserting its redacted representation
- Running the `doctor` command with secrets configured produces output that contains none of those secret values
- A CI-runnable grep or regex check prevents common regressions (for example, logging `Authorization` header values verbatim)

## Test plan
- New unit tests for redacted `Debug`/`Display` output on all secret-bearing config types
- Integration test that captures `tracing` output during a mocked Linear and OpenHands session and asserts absence of configured secret values
- Manual review pass of the final diff by a second maintainer given the sensitivity
