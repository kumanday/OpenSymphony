# Codex Subscription Support Roadmap Reshuffle

## Summary

Create a new near-term milestone for Codex + ChatGPT subscription readiness, retarget `COE-423` as the local/harness-orthogonal model settings seam, and move the new Codex milestone to `Todo`. Hosted mode remains intact, but hosted-only secret-store dependencies no longer block local Codex support.

## Key Linear Changes

- Create milestone `M10.3: Codex And Subscription Readiness`.
  - Goal: deliver local Codex app-server support and ChatGPT subscription credential foundations before full hosted mode.
  - Place it after M10 web client/external gateway work and before M10.5, M11, and the broader M12 hosted/provider backlog.

- Move to `M10.3` and set to `Todo`:
  - `COE-408 Harness Adapter And Capability Model`
  - `COE-423 Model And Credential Settings`
  - `COE-425 OpenHands Subscription Credential Adapter`
  - `COE-426 Codex App-Server Prototype And Benchmarks`
  - `COE-428 Model Configuration UI And Routing Metadata`
  - `COE-429 Codex Approvals And Cross-Harness Routing`

- Retarget `COE-423`:
  - Keep it as the shared model/credential settings task.
  - Narrow it to local and harness-orthogonal settings: API-key profiles, subscription credential references, local keychain/OpenHands auth-directory references, credential status, harness compatibility.
  - Remove the hosted secret-store implementation from its blocking path.

- Add new Backlog task in hosted lane:
  - Title: `Hosted Subscription Credential Broker And Secret Store`
  - Milestone: `M11: Hosted Alpha` or a new `M11.2: Hosted Credential Broker`
  - Scope: encrypted per-user/per-org ChatGPT OAuth refresh-token storage, token refresh, revocation, short-lived credential injection, audit logs, and tenant isolation.
  - Blocked by: `COE-420` and `COE-421`
  - Blocks: hosted production Codex use and hosted subscription-backed OpenHands use.

- Add new Backlog task in Codex lane:
  - Title: `Codex Production Harness Enablement`
  - Milestone: `M10.3: Codex And Subscription Readiness`
  - Scope: graduate the feature-gated prototype into a supported local harness: version detection, schema generation, JSON-RPC lifecycle, event normalization, cancellation/resume, error handling, and docs.
  - Blocked by: `COE-426`
  - Blocks: `COE-429`

- Add new Backlog task:
  - Title: `ChatGPT OAuth For Codex Harness`
  - Milestone: `M10.3: Codex And Subscription Readiness`
  - Scope: support `codex login --device-auth`/stored Codex credentials or equivalent supported auth path; expose login status, account identity where available, logout, and failure states.
  - Blocked by: `COE-423` and `COE-426`
  - Blocks: `Codex Production Harness Enablement`

- Move current hosted downstream Todo tasks back to `Backlog`:
  - `COE-421`, `COE-422`, `COE-424`, `COE-427`
  - Leave `COE-420` in `Human Review`.

## Dependency Changes

- Remove dependency: `COE-421 -> COE-423`.
  - Rationale: local model/credential settings should not require hosted secret storage.

- Preserve dependency: `COE-408 -> COE-423`.
  - Rationale: model settings need harness compatibility/capability concepts.

- Preserve dependency: `COE-408 -> COE-426`.
  - Rationale: Codex app-server should fit the shared adapter model.

- Keep or refine:
  - `COE-423 -> COE-425`
  - `COE-423 -> COE-426`
  - `COE-425 -> COE-428`
  - `COE-426 -> Codex Production Harness Enablement`
  - `ChatGPT OAuth For Codex Harness -> Codex Production Harness Enablement`
  - `Codex Production Harness Enablement + COE-428 -> COE-429`

- Add checked-in Linear query support before mutating dependencies:
  - Add `issue_relation_delete.graphql`.
  - Extend or add a dependency snapshot query that includes relation IDs.
  - Use those IDs to remove only the intended relation, then re-read the graph.

## Execution Order

1. Snapshot live project issues, milestones, states, and relation IDs.
2. Add missing repo-local Linear query files for relation deletion and relation-ID readback.
3. Create `M10.3: Codex And Subscription Readiness`.
4. Create the two new Codex/OAuth production tasks and the hosted credential-broker task.
5. Update `COE-423` description and milestone assignment.
6. Move the selected Codex tasks to `M10.3`.
7. Move all `M10.3` tasks to `Todo`.
8. Move `COE-421`, `COE-422`, `COE-424`, and `COE-427` to `Backlog`.
9. Delete only the `COE-421 blocks COE-423` relation.
10. Add the new blocker relations listed above.
11. Re-read Linear and verify milestone membership, states, and dependency graph.
12. Update project overview content if it has a dependency-priority table.

## Tests And Verification

- Live Linear read-back must show:
  - `M10.3` exists.
  - All moved Codex tasks are in `M10.3` and `Todo`.
  - Hosted downstream tasks are back in `Backlog`.
  - `COE-423` is no longer blocked by `COE-421`.
  - `COE-426` remains blocked by `COE-408` and `COE-423`.
  - `COE-429` is blocked by the production Codex task and `COE-428`.

- Repo verification:
  - If query files are added, run a non-mutating syntax/read smoke check by invoking the helper against a known read-only query.
  - No code build is required unless repo docs/query assets are changed beyond GraphQL helper files.

## Assumptions

- The visible Linear project slug is `opensymphony-bootstrap-e7b957855cb7`; the GraphQL `Project.slugId` currently used by the helper is `e7b957855cb7`.
- `Todo` is acceptable for the whole new Codex milestone even though later tasks remain dependency-blocked; the scheduler/UI should still respect blocker relations.
- Full hosted multi-tenant ChatGPT OAuth support is not required before local Codex subscription support, but it must be explicitly tracked as a hosted production gap.
