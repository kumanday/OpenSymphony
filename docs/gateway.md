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
