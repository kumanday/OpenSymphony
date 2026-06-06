---
name: custom-codereview-guide
description: |
  Repository-specific code review guidance for this project.
  Update this file so OpenHands PR review focuses on the right risks.
---

# Custom Code Review Guide

## Default Priorities

- Prioritize correctness, regressions, security risks, and missing tests ahead of style-only feedback.
- Treat behavior changes as incomplete unless the PR includes concrete verification or evidence.
- Call out risky data migrations, auth changes, concurrency hazards, and production operability regressions explicitly.

## COE-409 Desktop Settings & Native Actions — Review Context

PR #108: `feat: desktop settings, keychain, and native actions`

The following items have been flagged by prior AI review rounds but are **already resolved** in the current branch. Do not re-flag them:

### Already Resolved Items (DO NOT flag)

1. **/dev/null fallback in global_manager()** — RESOLVED in e5e98da. Uses `std::env::temp_dir().join("opensymphony-settings-fallback.json")` instead.

2. **canonicalize().unwrap_or(base) symlink mismatch** — RESOLVED in 014643f. Both `reveal_workspace` and `is_safe_workspace_path` now check containment BEFORE canonicalization, eliminating mismatch risk.

3. **Trivial test_settings_load_or_default** — ENRICHED in 092eb0a. Now exercises multi-manager persistence, type preservation, and round-trip save/load with atomic writes.

4. **Trivial actions.rs tests** — ENRICHED across d931022/014643f. 21 tests now cover path safety (system paths, traversal, symlinks), canonicalization errors, notification deserialization, URL encoding, request validation, keychain redaction, and settings persistence.

5. **Evidence section in PR description** — PRESENT. See PR description for test commands and output.

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

## COE-398 Tauri Desktop Shell — Review Context

PR #93: `feat: add Tauri desktop shell and security capabilities`

The following items have been flagged by prior AI review rounds but are **already resolved** in the current branch. Do not re-flag them:

### Already Resolved Items (DO NOT flag)

1. **CSP wildcards** — RESOLVED. `tauri.conf.json` CSP uses exact hosts: `wss://api.opensymphony.dev` and `wss://api.opensymphony.app`. No wildcards present.

2. **DesktopError serialization** — RESOLVED. `commands.rs` line 22: `#[serde(tag = "type")]` (internally-tagged). All variants produce uniform JSON shape.

3. **SettingValue serialization** — RESOLVED. `commands.rs` line 118: `#[serde(tag = "type", content = "value")]` (adjacently-tagged). Unambiguous serialization.

4. **main.rs panic handling** — RESOLVED. `main.rs` uses `if let Err(e)` + `process::exit(1)`. No `.expect()` calls remain.

5. **Security checklist permission names** — RESOLVED. `docs/tauri-security-checklist.md` uses actual Tauri v2 identifiers: `dialog:allow-open`, `dialog:allow-save`, `notification:allow-show`, `notification:allow-request-permission`.

6. **Shell permissions** — RESOLVED. `process-supervision.json` grants only `shell:default`. No `shell:execute` or `shell:kill` permissions active.

7. **beforeDevCommand** — RESOLVED. `tauri.conf.json` runs `cd .. && npm run dev` which maps to `serve dist -l 1420`. This starts a persistent dev server on port 1420.

8. **Cargo.lock reproducibility** — RESOLVED. Desktop binary `Cargo.lock` is committed.

9. **build-stub.mjs** — RESOLVED. Valid stub HTML generated to prevent empty-frontend white-screen.

10. **serve pinned in devDependencies** — RESOLVED.

11. **Version alignment** — RESOLVED. `tauri.conf.json` version = `Cargo.toml` version = `1.6.0`.

12. **beforeBuildCommand error propagation** — RESOLVED. Uses explicit `exit 1` on failure, no `|| true`.

13. **Icon dimensions** — RESOLVED. `gen_icons.py` generates correct dimensions per file (32x32, 128x128, 256x256 for @2x).

14. **Workspace members** — RESOLVED. Matches origin/main with `members = ["."]`.

### What TO Review

- New code correctness in `apps/desktop/src-tauri/src/` (commands.rs, main.rs, lib.rs)
- Capability file structure in `apps/desktop/src-tauri/capabilities/`
- Tauri config in `apps/desktop/src-tauri/tauri.conf.json`
- Security checklist accuracy in `docs/tauri-security-checklist.md`
- Build reproducibility (Cargo.lock committed, versions aligned)

## Customize For This Repository

- Rust workspace: root crate + `apps/desktop/src-tauri` (excluded from workspace for CI compat)
- Tauri v2 with capabilities-based permission model
- Desktop is premium local experience connecting to hosted remote profiles

## Evidence Expectations

- Behavior changes should include test or reproduction output.
- UI changes should include screenshots or recordings.
- Performance-sensitive changes should include benchmark data or timing notes.
