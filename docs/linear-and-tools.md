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

Current workflow contract:

- `tracker.kind` must be `linear`
- `tracker.project_slug` stores Linear `Project.slugId`
- `LINEAR_API_KEY` must be available when Linear mode is enabled

Important normalization rules:

- `blocked_by` is derived from `inverseRelations` entries whose relation type is
  `blocks`
- `parent_id` comes from `parent.id`
- `sub_issues` comes from `children.nodes`
- `state` remains the workflow-facing state name string used by
  `WORKFLOW.md`

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
