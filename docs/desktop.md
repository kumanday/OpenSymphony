---
type: topic-doc
area: desktop
visibility: public
last_memory_sync: 2026-06-21T19:06:47.514408+00:00
---

# Desktop

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-551 contributed: PR #232: [COE-551] Scope cross-repository memory and leaf overlays (merge `c907f6f`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-551: Scoped Cross-Repository Memory And Leaf Overlays

## Source refs

- COE-551

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
## Local Build And Run

The `opensymphony app` command, with visible alias `opensymphony desktop`, is
implemented by COE-488 / OSYM-811 as a lazy cached launcher. It verifies a
versioned desktop bundle under `~/.opensymphony/desktop/<version>/` and can
materialize an early local bundle from `--bundle-dir <path>` or
`OPENSYMPHONY_DESKTOP_BUNDLE_DIR` without adding Tauri or npm dependencies to
the normal Cargo install path. `--install-path <dir>` selects a custom install
root with versioned bundles beneath it. When no verified bundle is cached, the
launcher downloads the schema version 1 release index from the versioned GitHub
release for the running CLI, verifies the compatible archive, promotes it into
the versioned cache, and launches from there. Existing cached bundles check the
latest GitHub release index for newer compatible updates before launch. Set
`OPENSYMPHONY_DESKTOP_RELEASE_INDEX_URL` to use a private mirror or fake release
server.
If the release index has no compatible prebuilt asset, or the index/archive
download is unavailable, the launcher falls back to a matching source build
after prerequisite checks. Interactive updates prompt with
`Update before launch? [Y/n]`; Enter means yes, and non-interactive launches
update by default unless `--no-update` is supplied.

For development from a clone, rebuild the desktop frontend when `apps/desktop`
or shared frontend packages change:

```bash
npm ci --include=dev
```

Then launch the Tauri shell:

```bash
cd apps/desktop/src-tauri
cargo run
```

`cargo run` rebuilds the desktop frontend first, so local source changes under
`apps/desktop` and shared frontend packages are reflected in the Tauri shell.

For frontend hot reload, run `npm run dev --workspace=@opensymphony/desktop`
in one terminal and `cargo run` from `apps/desktop/src-tauri` in another.
Plain `cargo run` rebuilds and reads the current `apps/desktop/dist` bundle; it
does not start Vite.

## Code Graph production smoke

For an empty Code Graph store, launch the normal desktop shell, open Graph, and
choose the configured repository's `Index repository` action. The status panel
shows target-branch progress, parsed/persisted coverage, skipped files, and
retry diagnostics; completion is driven by `code_graph_updated`, with a
repository-summary polling fallback while accepted/progress jobs have no event
delivery. This path uses the production Tauri command, not fixture data. The
equivalent web smoke uses the production HTTP adapter. A packaged parity check
is:

```bash
npm run build --workspace=@opensymphony/desktop
npm run package:release --workspace=@opensymphony/desktop -- --dry-run
```

The installed bundle must contain the same shared shell and production adapter
selection as the local Tauri launch; `?fixtures` is reserved for the graph
visualization workbench.

## Release Bundle

The first release asset format is a tarball consumed by the lazy CLI launcher.
Build and package it from the repository root:

```bash
npm run build --workspace=@opensymphony/desktop
npm run package:release --workspace=@opensymphony/desktop
```

The package step writes a stable archive name and release index under
`dist/desktop-release/`. The archive includes
`opensymphony-desktop-manifest.json` plus the launch target named in that
manifest. If an index already exists, the package step keeps other
platform/architecture entries and replaces only the current asset entry. Upload
the archive before replacing `opensymphony-desktop-release-index.json`.

## Task Graph Project Groups

When the gateway task graph includes Linear project metadata, the desktop task
graph groups visible issue rows under compact project headings. Operators can
collapse a project group for the current session; collapsed state is local to
the client and does not clear the currently selected run detail. Cross-project
dependency detail remains in the selected run detail pane; the compact grouped
list keeps dependency lines local to each project group.
