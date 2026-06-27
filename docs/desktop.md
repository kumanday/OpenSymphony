---
type: topic-doc
area: desktop
visibility: public
last_memory_sync: 2026-06-21T19:06:47.514408+00:00
---

# Desktop

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-460 contributed: PR #148: Add OKF import and export CLI (merge `add8410`)
- COE-463 contributed: PR #149: Add OKF memory admin MCP parity (merge `f5f4809`)
- COE-479 contributed: PR #138: Resume Codex debug sessions (merge `0841adb`)
- COE-480 contributed: PR #140: Show truthful run detail metrics (merge `ec2dc04`)
- COE-481 contributed: PR #139: Refresh OpenAI model profile defaults (merge `0f2a74f`)
- COE-482 contributed: PR #143: Fix Codex token usage accounting in TUI (merge `d48ad13`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-460: OKF Export, Import, And Visibility Boundaries
- COE-463: Docs Sync And MCP Admin Parity For OKF
- COE-479: Codex Debug Session Resume
- COE-480: Run Detail Metrics And Density
- COE-481: Model Configuration Codex Subscription Follow-Up
- COE-482: TUI Codex Token Usage Accounting
- COE-483: Codex Event Content Summaries
- COE-484: Desktop Live Snapshot And Run Detail Refresh
- COE-485: Harden desktop live event resumption and refresh failure visibility
- COE-494: Project Metadata For Operator Issue Snapshots
- COE-504: Linear Polling And Rate-Limit Recovery

## Source refs

- COE-460
- COE-463
- COE-479
- COE-480
- COE-481
- COE-482
- COE-483
- COE-484
- COE-485
- COE-494
- COE-504

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
## Local Build And Run

The desktop app currently requires a repo checkout. The planned
`opensymphony app` / `opensymphony desktop` lazy installer is tracked by
COE-488 / OSYM-811 and is not implemented yet.

From a clone, rebuild the desktop frontend when `apps/desktop` or shared
frontend packages change:

```bash
npm install
npm run build --workspace=@opensymphony/desktop
```

Then launch the Tauri shell:

```bash
cd apps/desktop/src-tauri
cargo run
```

The workspace build command is the same as running `npm run build` from
`apps/desktop`. If you are already in `apps/desktop/src-tauri`, use
`npm --prefix .. run build`.

For frontend hot reload, run `npm run dev --workspace=@opensymphony/desktop`
in one terminal and `cargo run` from `apps/desktop/src-tauri` in another.
Plain `cargo run` uses the current `apps/desktop/dist` bundle and does not
start Vite.

## Task Graph Project Groups

When the gateway task graph includes Linear project metadata, the desktop task
graph groups visible issue rows under compact project headings. Operators can
collapse a project group for the current session; collapsed state is local to
the client and does not clear the currently selected run detail. Cross-project
dependency detail remains in the selected run detail pane; the compact grouped
list keeps dependency lines local to each project group.
