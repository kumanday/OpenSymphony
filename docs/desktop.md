---
type: topic-doc
area: desktop
visibility: public
last_memory_sync: 2026-06-21T19:06:47.514408+00:00
---

# Desktop

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-494 contributed: PR #150: Expose project metadata in operator snapshots (merge `67f973a`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-494: Project Metadata For Operator Issue Snapshots

## Source refs

- COE-494

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->

## Task Graph Project Groups

When the gateway task graph includes Linear project metadata, the desktop task
graph groups visible issue rows under compact project headings. Operators can
collapse a project group for the current session; collapsed state is local to
the client and does not clear the currently selected run detail. Cross-project
dependency detail remains in the selected run detail pane; the compact grouped
list keeps dependency lines local to each project group.
