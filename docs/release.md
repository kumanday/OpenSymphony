---
type: topic-doc
area: release
visibility: public
last_memory_sync: 2026-07-04T03:35:18.210566+00:00
---

# Release

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-546 contributed: PR #217: Code Graph bootstrap indexing UX and E2E validation (merge `5cc1e83`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-546: Code Graph Bootstrap UX And End-To-End Validation

## Source refs

- COE-546

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
