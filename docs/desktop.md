---
type: topic-doc
area: desktop
visibility: public
last_memory_sync: 2026-06-21T19:06:47.514408+00:00
---

# Desktop

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-524 contributed: PR #185: Add workflow settings update mode (merge `7dede06`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-524: Template Docs And Settings Hardening

## Source refs

- COE-524

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
## Local Build And Run

The `opensymphony app` command, with visible alias `opensymphony desktop`, is
implemented by COE-488 / OSYM-811 as a lazy cached launcher. It verifies a
versioned desktop bundle under `~/.opensymphony/desktop/<version>/` and can
materialize an early local bundle from `--bundle-dir <path>` or
`OPENSYMPHONY_DESKTOP_BUNDLE_DIR` without adding Tauri or npm dependencies to
the normal Cargo install path. `--install-path <dir>` selects a custom install
root with versioned bundles beneath it. Signed or downloaded bundle
distribution remains future installer work.

For development from a clone, rebuild the desktop frontend when `apps/desktop`
or shared frontend packages change:

```bash
npm install
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

## Task Graph Project Groups

When the gateway task graph includes Linear project metadata, the desktop task
graph groups visible issue rows under compact project headings. Operators can
collapse a project group for the current session; collapsed state is local to
the client and does not clear the currently selected run detail. Cross-project
dependency detail remains in the selected run detail pane; the compact grouped
list keeps dependency lines local to each project group.
