# Linear and Tools

## 1. Boundary

OpenSymphony uses Linear in two different ways:

- the Rust orchestrator reads Linear through the internal `opensymphony_linear`
  module
- the coding agent reads and writes Linear through the repo-local GraphQL skill
  assets copied into the target repository

Scheduler correctness must never depend on agent-side ticket writes succeeding.

## 2. Orchestrator read adapter

The internal `opensymphony_linear` module is the only tracker adapter used by
the daemon.

It is responsible for:

- fetching active candidates for the configured project
- refreshing current state for already-running issues
- reading terminal issues for startup cleanup
- normalizing GraphQL payloads into stable domain models
- serving gateway task graph reads with Linear-native parent, child, and
  blocker relationships
- loading gateway task graph issue details through project-scoped, paged Linear
  reads rather than per-identifier GraphQL lookups

Current workflow contract:

- `tracker.kind` must be `linear`
- `tracker.project_slug` stores Linear `Project.slugId`
- `LINEAR_API_KEY` must be available when Linear mode is enabled
- the scheduler's Linear tracker client remains mandatory for `opensymphony run`;
  the gateway task graph reader is optional and, when unavailable, causes only
  the task graph endpoint to return `503`

Important normalization rules:

- `blocked_by` is derived from `inverseRelations` entries whose relation type is
  `blocks`; gateway task graph responses filter these IDs to nodes present in
  the returned project snapshot so clients do not receive dangling graph edges
- `state_kind` is derived from Linear's stable workflow-state `type`; clients and
  caches must not infer categories from mutable display names such as
  "Human Review"
- `parent_id` comes from `parent.id`
- `parent` retains the parent identifier when Linear returns it, and gateway
  task graph nodes use that identifier as the client-facing `parent_id`; the
  gateway clears `parent_id` when that parent is outside the returned project
  snapshot so clients do not receive dangling hierarchy edges
- `sub_issues` comes from `children.nodes`
- gateway task graph `children` are filtered to nodes present in the returned
  project snapshot
- `state` remains the workflow-facing state name string used by
  `WORKFLOW.md`
- gateway task graph `root_ids` are the returned node identifiers whose Linear
  parent is absent or outside the returned node set; clients must not infer
  tracker hierarchy from fixture data or local fallbacks

## 3. Agent-side Linear access

OpenSymphony 1.0.0 is GraphQL-only for agent-side Linear work.

Every initialized repository receives:

- `.agents/skills/linear/SKILL.md`
- `.agents/skills/linear/scripts/linear_graphql.py`
- `.agents/skills/linear/queries/*.graphql`
- `.agents/skills/linear/references/*.md`

Later, `opensymphony update` refreshes the template-managed `.agents/skills/`
tree in place for an existing target repo without rerunning the full bootstrap
flow.

The agent path is intentionally simple:

1. require `LINEAR_API_KEY`
2. choose a checked-in query file
3. pass variables as JSON
4. inspect the returned JSON

Example:

```bash
python3 .agents/skills/linear/scripts/linear_graphql.py \
  --query-file .agents/skills/linear/queries/issue_by_key.graphql \
  --variables '{"key":"COE-123"}'
```

## 4. Supported GraphQL workflows

The checked-in query assets cover the current repository-supported write and
inspection paths:

- issue create and follow-up issue creation
- issue body and metadata updates
- issue lookup by key or ID
- issue detail reads
- team workflow-state lookup
- issue transitions
- comment create and update
- issue relation creation
- GitHub PR attachment
- plain URL attachment
- project lookup by slug
- project overview/content updates
- project status create, update, and assignment
- upload bootstrapping through `fileUpload`
- schema introspection for mutation names and input shapes

If a new mutation is needed, prefer adding a checked-in query file and updating
the skill references instead of improvising large inline GraphQL strings in
prompts.

## 5. Why GraphQL-only

OpenSymphony previously carried a custom Linear bridge layer for agent-side
writes. That indirection is gone in 1.0.0.

The GraphQL-only design keeps the system smaller and easier to reason about:

- no extra local bridge process
- no duplicated tool contract to maintain
- no ambiguity about which Linear surface the agent should use
- full access to Linear capabilities without waiting for a narrower wrapper

## 6. Failure model

The expected behavior is:

- missing `LINEAR_API_KEY` is a real blocker for Linear operations
- GraphQL write failures do not change scheduler correctness
- the orchestrator continues to reconcile issue state from its own read adapter
- target-repo skills must treat a top-level GraphQL `errors` array as failure
- `opensymphony linear archive` is an operator command, not an agent-side write;
  it refuses to archive issues without fresh captured memory unless `--force`
  is supplied

## 7. Repository ownership

The relevant ownership boundaries are:

- `crates/opensymphony-linear/`
  - orchestrator-side GraphQL adapter module tree
- `crates/opensymphony-workflow/`
  - workflow validation module tree for Linear-related config
- `.agents/skills/linear/` in the template repo
  - agent-side GraphQL helper, query files, and references

OpenSymphony intentionally does not ship a second agent-side Linear server.

## 8. Validation

Before merging Linear-related changes:

- run `cargo test`
- run `cargo test --test init`
- run `cargo test --test update`
- initialize a sample repo with `opensymphony init`
- confirm the copied `.agents/skills/linear/` tree includes scripts, queries,
  and references
- update the same sample repo with `opensymphony update` and confirm changed or
  new template-managed Linear skill files sync cleanly
- smoke-test the helper with `queries/viewer.graphql`

## 9. Migration note

OpenSymphony 1.0.0 removed workflow-owned Linear bridge configuration.

If an older repository still contains `openhands.mcp`, remove that block and
use the repo-local Linear GraphQL helper assets with `LINEAR_API_KEY` instead.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-254 contributed: PR #6: COE-254: bootstrap tracker, workspace, and orchestration core
- COE-263 contributed: PR #35: COE-263: Implement workspace manager and lifecycle hooks (merge `2693eea`)
- COE-264 contributed: PR #33: COE-264: Linear read adapter and issue normalization (merge `45cca3c`)
- COE-267 contributed: PR #83: Add memory init and mapped docs sync
- COE-268 contributed: PR #43: Implement orchestrator scheduler retries and reconciliation (merge `2ad73ad`)
- COE-270 contributed: PR #39: COE-270: add deterministic workspace context artifacts (merge `3a90eea`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-254: Tracker, Workspaces, and Orchestration
- COE-263: Workspace manager and lifecycle hooks
- COE-264: Linear read adapter and issue normalization
- COE-267: Linear MCP write surface
- COE-268: Orchestrator scheduler, retries, and reconciliation
- COE-270: Repository harness and generated context artifacts
- COE-277: Implement hierarchy-aware task selection

## Source refs

- COE-254
- COE-263
- COE-264
- COE-267
- COE-268
- COE-270
- COE-277

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
