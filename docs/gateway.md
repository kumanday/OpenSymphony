---
type: topic-doc
area: gateway
visibility: public
last_memory_sync: 2026-06-21T19:11:22.242702+00:00
---

# Gateway

## Code Graph repository indexing

The repository index route is a server-side, bounded job:

```text
POST /api/v1/code/repos/{repo_id}/index
```

It resolves the configured repository and `WORKFLOW.md` target branch, reads
the target commit's Git tree/blob objects, and persists an immutable snapshot.
The request does not accept a filesystem root and indexing never executes
repository code. A first response is `accepted`; background progress and the
terminal `completed`, `unavailable`, or `failed` result are written to the event
journal. Completion also emits `code_graph_updated`. Re-indexing a later commit
parses only changed/added paths, records deletions as stale current rows, and
retains older snapshot membership for revision queries.

The Tauri `code_index_repo` command is a thin native mirror of this route and
uses the same DTOs and event behavior. The persistent index is a target-branch
baseline, not an issue-workspace overlay.

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
