---
type: topic-doc
area: release
visibility: public
last_memory_sync: 2026-07-04T03:35:18.210566+00:00
---

# Release

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-531 contributed: PR #209: feat(graph): add Code Graph frontend surface (merge `b6fcf57`)
- COE-532 contributed: PR #201: fix(orchestrator): stop reporting parked recovered issues as completed (merge `f6cddee`)
- COE-533 contributed: PR #209: feat(graph): add Code Graph frontend surface (merge `b6fcf57`)
- COE-534 contributed: PR #209: feat(graph): add Code Graph frontend surface (merge `b6fcf57`)
- COE-535 contributed: PR #210: feat(run-detail): add diff symbol navigation (merge `863e523`)
- COE-536 contributed: PR #211: feat(graph): connect cross-graph code chips (merge `f815ca2`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-531: Workspace Shell Graph Hero And Surface State
- COE-532: Symbol Identity Container Chain And Code Read Model
- COE-533: Code Graph DTOs Gateway Routes And Native Commands
- COE-534: Code Graph Frontend Surface Adapters And Inspector
- COE-535: Run Diff Symbol Navigation And Code Overlay
- COE-536: Cross Graph Code Memory And Work Chips
- COE-537: Code Graph Scale Accessibility And Parity Hardening
- COE-540: Canonical Codex Thread Reuse And Workspace Retention
- COE-541: Durable Codex Thread Archive And Debug Recovery

## Source refs

- COE-531
- COE-532
- COE-533
- COE-534
- COE-535
- COE-536
- COE-537
- COE-540
- COE-541

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->

## M12.9 Code Graph desktop release gate

The Code Graph follow-on is release-ready only when the web and packaged
desktop shells use the production HTTP/native adapters and the empty-database
index action is present in the installed bundle. Before publishing, verify
that the root crate and desktop metadata remain in lock-step across
`Cargo.toml`, `apps/desktop/package.json`, `apps/desktop/src-tauri/Cargo.toml`,
`apps/desktop/src-tauri/Cargo.lock`, `apps/desktop/src-tauri/tauri.conf.json`,
and the root `package-lock.json`.

Use the default bundled-mode checks plus the package parity guard:

```bash
cargo clippy --all-targets -- -D warnings
cargo test
npm run build --workspace=@opensymphony/desktop
npm run package:release --workspace=@opensymphony/desktop -- --dry-run
```

The dry run must pass without creating a release archive. The desktop smoke
must launch the packaged bundle against a real gateway and verify repository
indexing, progress/error/retry rendering, completion refresh, and target
revision provenance; fixture mode is only a visualization-workbench check.
