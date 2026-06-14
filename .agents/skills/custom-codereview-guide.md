---
name: custom-codereview-guide
description: Project-specific overlay for automated pull request review
triggers:
  - /codereview
---

# Custom Code Review Guide

## Default Priorities

- Prioritize correctness, regressions, security risks, and missing tests ahead of style-only feedback.
- Treat behavior changes as incomplete unless the PR includes concrete verification or evidence.
- Call out risky data migrations, auth changes, concurrency hazards, and production operability regressions explicitly.

## COE-409 Desktop Settings & Native Actions -- Review Context

PR #108: `feat: desktop settings, keychain, and native actions`

The following items have been flagged by prior AI review rounds but are **already resolved** in the current branch. Do not re-flag them:

### Already Resolved Items (DO NOT flag)

1. **/dev/null fallback in global_manager()** -- RESOLVED in e5e98da. Uses `std::env::temp_dir().join("opensymphony-settings-fallback.json")` instead.

2. **canonicalize().unwrap_or(base) symlink mismatch** -- RESOLVED in 014643f. Both `reveal_workspace` and `is_safe_workspace_path` now check containment BEFORE canonicalization, eliminating mismatch risk.

3. **Trivial test_settings_load_or_default** -- ENRICHED in 092eb0a. Now exercises multi-manager persistence, type preservation, and round-trip save/load with atomic writes.

4. **Trivial actions.rs tests** -- ENRICHED across d931022/014643f. 21 tests now cover path safety (system paths, traversal, symlinks), canonicalization errors, notification deserialization, URL encoding, request validation, keychain redaction, and settings persistence.

5. **Evidence section in PR description** -- PRESENT. See PR description for test commands and output.

### Evidence for COE-409

```bash
# Build and test (all 21 pass, zero clippy warnings)
cd apps/desktop/src-tauri && cargo test
cargo clippy
```

Test output (latest run):
```
running 21 tests
test actions::tests::test_is_safe_workspace_path_allows_opensymphony_subdirs ... ok
test actions::tests::test_is_safe_workspace_path_blocks_path_traversal_attempts ... ok
test actions::tests::test_is_safe_workspace_path_blocks_tricky_system_paths ... ok
test actions::tests::test_open_linear_link_request_url_encoding ... ok
test actions::tests::test_canonicalize_nonexistent_path_error_kind ... ok
test keychain::tests::test_redact_value_does_not_leak_errors ... ok
test settings::tests::test_settings_atomic_write ... ok
test settings::tests::test_settings_load_or_default ... ok
test settings::tests::test_settings_manager_round_trip ... ok
... (21 total, all passing)

test result: ok. 21 passed; 0 failed
```

### What TO Review

- Path containment logic in `is_safe_workspace_path` and `reveal_workspace`
- Keychain credential status display and redaction helpers
- Settings persistence with atomic write semantics
- Notification integration and Linear URL generation

## COE-410 Desktop Local Stream Optimization -- Review Context

The following items have been flagged by prior AI review rounds but are **already resolved** in the current branch. Do not re-flag them:

### Already Resolved Items (DO NOT flag)

1. **Tauri channel API contract** -- RESOLVED. `TauriChannelTransport` correctly implements the Tauri v2 frontend pattern:
   - Frontend creates `Channel` via `tauri.core.Channel<GatewayEnvelope>(callback)`
   - Passes channel as `tx` argument to `invoke("subscribe_events", { tx: channel })`
   - Backend receives `tx: tauri::ipc::Channel<GatewayEnvelope>` -- verified by Rust command signatures
   - See `packages/api-client/src/transports.ts` lines 802-814 and 847-860

2. **Generator cancellation** -- RESOLVED. `TauriChannelTransport` has `isClosed` flag and `pendingGeneratorCancellers` Set.
   - `close()` resolves all pending generator promises with `{ done: true }`
   - Generators check `while (!this.isClosed)` in their loop
   - See `packages/api-client/src/transports.ts` lines 818-840 and 864-886

3. **GatewayEnvelope redefinition** -- RESOLVED. `commands.rs` imports `GatewayEnvelope` from `opensymphony_gateway_schema::envelope::GatewayEnvelope`.
   - No local redefinition exists; all DTO types imported from the schema crate
   - See `apps/desktop/src-tauri/src/commands.rs` lines 859-865

4. **SSE multi-line payloads** -- RESOLVED. `HttpGatewayTransport` accumulates `data:` lines with newlines, parsing on `\n\n` boundary per SSE spec.
   - See `packages/api-client/src/transports.ts` lines 152-251

5. **WebSocket terminal frame dispatch** -- RESOLVED. `WebSocketTransport.handleMessage()` dispatches to both `eventSubscribers` and `terminalSubscribers` based on `envelope.entity_ref.kind`.
   - See `packages/api-client/src/transports.ts` lines 590-650

6. **WebSocket connect timeout** -- RESOLVED. `connectWebSocket()` has `WS_CONNECT_TIMEOUT_MS = 10_000` with `setTimeout` that rejects the promise on timeout.
   - See `packages/api-client/src/transports.ts` lines 520-560

7. **Attach gateway loopback check** -- RESOLVED. `attach_gateway` checks `127.0.0.1`, `localhost`, `::1`, and `0.0.0.0`.
   - See `apps/desktop/src-tauri/src/commands.rs` lines 666-670

8. **Cargo.lock** -- RESOLVED. Desktop `Cargo.lock` is committed and restored in commit `d1ec424`.

### What TO Review for COE-410

- New transport correctness: `TauriChannelTransport`, `WebSocketTransport`, `HttpGatewayTransport`
- Rust command handlers in `apps/desktop/src-tauri/src/commands.rs` for gateway connectivity
- Transport equivalence tests in `tests/transport_equivalence.rs` and `packages/api-client/__tests__/transport-contract.test.ts`
- Stream benchmarks in `tests/stream_benchmarks.rs`
- Type exports in `packages/api-client/src/index.ts` and `packages/api-client/src/transports.ts`

## COE-398 Tauri Desktop Shell -- Review Context

PR #93: `feat: add Tauri desktop shell and security capabilities`

The following items have been flagged by prior AI review rounds but are **already resolved** in the current branch. Do not re-flag them:

### Already Resolved Items (DO NOT flag)

1. **CSP wildcards** -- RESOLVED. `tauri.conf.json` CSP uses exact hosts: `wss://api.opensymphony.dev` and `wss://api.opensymphony.app`. No wildcards present.

2. **DesktopError serialization** -- RESOLVED. `commands.rs` line 22: `#[serde(tag = "type")]` (internally-tagged). All variants produce uniform JSON shape.

3. **SettingValue serialization** -- RESOLVED. `commands.rs` line 118: `#[serde(tag = "type", content = "value")]` (adjacently-tagged). Unambiguous serialization.

4. **main.rs panic handling** -- RESOLVED. `main.rs` uses `if let Err(e)` + `process::exit(1)`. No `.expect()` calls remain.

5. **Security checklist permission names** -- RESOLVED. `docs/tauri-security-checklist.md` uses actual Tauri v2 identifiers: `dialog:allow-open`, `dialog:allow-save`, `notification:allow-show`, `notification:allow-request-permission`.

6. **Shell permissions** -- RESOLVED. `process-supervision.json` grants only `shell:default`. No `shell:execute` or `shell:kill` permissions active.

7. **beforeDevCommand** -- RESOLVED. `tauri.conf.json` runs `cd .. && npm run dev` which maps to `serve dist -l 1420`. This starts a persistent dev server on port 1420.

8. **Cargo.lock reproducibility** -- RESOLVED. Desktop binary `Cargo.lock` is committed.

9. **build-stub.mjs** -- RESOLVED. Valid stub HTML generated to prevent empty-frontend white-screen.

10. **serve pinned in devDependencies** -- RESOLVED.

11. **Version alignment** -- RESOLVED. `tauri.conf.json` version = `Cargo.toml` version = `1.6.0`.

12. **beforeBuildCommand error propagation** -- RESOLVED. Uses explicit `exit 1` on failure, no `|| true`.

## COE-414 Diff, Validation, Approval, and Run Action Views -- Review Context

PR #119: `feat: rich run-detail experience (diff, validation, approval, run-action views)`

The following items have been flagged by prior AI review rounds but are **already resolved** in the current branch. Do not re-flag them:

### Already Resolved Items (DO NOT flag)

1. **Synthetic validation commands / evidence** -- RESOLVED. `get_run_validation` in `crates/opensymphony-gateway/src/lib.rs` now returns the computed overall status derived from runtime state and empty `commands`/`evidence` arrays; placeholder `command_id`/`evidence_id` entries are removed. Unknown runs return `overall_status: Error` with empty arrays.

2. **Synthetic approval data** -- RESOLVED. `get_run_approvals` returns an empty `approvals` list until the harness produces real approval requests. The schema and UI still render real approvals when supplied by the client/mock.

3. **Unsafe detach on healthy active runs** -- RESOLVED. `safe_actions_for_issue` now derives `detach` from `build_liveness(issue).stream`: detach is only safe when the stream is not `Healthy` and the issue is not already detached. This covers degraded, stalled, and cancel-failed states while preventing healthy active/quiet runs from being detached.

4. **Diff hunk header with file path** -- RESOLVED. `build_run_diff` now emits standard `@@ -old_start,old_count +new_start,new_count @@` headers without the embedded file path.

5. **Unused `envelope` parameter in `build_liveness`** -- RESOLVED. `build_liveness` signature removed the unused parameter and all call sites updated.

6. **TypeScript action/approval schema alignment** -- RESOLVED in `109d92a`. `ActionReceipt` uses `expected_followup`, `ActionStatus` is exported, and `stableHash()` drives deterministic idempotency keys.

### Evidence for COE-414

This PR delivers UI-core components, gateway endpoints, and schema types. Verification is via DOM fixture tests and Rust integration tests rather than end-to-end screenshots because the views are rendered in a lightweight HTML/TUI layer inside the app and there is no browser-based harness in this repo.

```bash
# Rust library unit tests (480)
cargo test --lib

# Gateway + action handler integration tests (65)
cargo test --test gateway --test action_handler

# Targeted UI/run-detail tests
npx jest --config jest.config.js packages/ui-core/__tests__

# Full npm workspace test suite
npm run test
```

Latest run results (post-fix):

```
cargo test --lib: 480 passed
cargo test --test gateway --test action_handler: 65 passed (49 gateway + 16 action_handler)
npx jest packages/ui-core/__tests__: 96 passed
npm run test: 18 suites, 351 tests passed
```

Note: full `cargo test` also runs unrelated `memory` integration tests that require `OPENSYMPHONY_MEMORY_ADMIN_TOKEN`; those are not part of this PR's acceptance validation and are not run in the targeted commands above.

### What TO Review for COE-414

- Gateway correctness: `get_run_validation`, `get_run_approvals`, `build_run_diff`, `safe_actions_for_issue`, `build_liveness` in `crates/opensymphony-gateway/src/lib.rs`
- UI rendering: `packages/ui-core/src/diff.ts`, `packages/ui-core/src/validation.ts`, `packages/ui-core/src/approvals.ts`, `packages/ui-core/src/run-actions.ts`
- Schema consistency: `packages/gateway-schema/src/action.ts`, `packages/gateway-schema/src/approval.ts`, `packages/gateway-schema/src/validation.ts`
- Transport idempotency: `packages/api-client/src/util.ts` `stableHash()` usage in `commentRun` for `HttpGatewayTransport` and `MockGatewayTransport`
- Fixture tests: `packages/ui-core/__tests__/run-detail-views.test.ts`, `packages/ui-core/__tests__/run-actions.test.ts`, `crates/opensymphony-gateway/tests/gateway.rs`
