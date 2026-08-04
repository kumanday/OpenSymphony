---
type: topic-doc
area: release
visibility: public
last_memory_sync: 2026-07-04T03:35:18.210566+00:00
---

# Release

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-549 contributed: PR #231: Verified checkout generations and harness envelopes (merge `a757a7d`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-549: Verified Checkouts Instructions And Harness Envelopes

## Source refs

- COE-549

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
## 2.11 Rust toolchain boundary

OpenSymphony 2.11.0 requires Rust 1.97.1 for the root CLI and desktop crate and
uses Cargo Resolver 3. The repository toolchain, package `rust-version`, Clippy
MSRV, CI toolchain, and desktop metadata must remain aligned.

Users upgrading from a CLI older than 2.11 may need to bypass an older
checkout-local toolchain override:

```bash
rustup update stable
cargo +stable install opensymphony --locked
```

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
