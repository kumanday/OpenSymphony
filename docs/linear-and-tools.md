# Linear and Tools

Linear is the default tracker. Jira is also supported for the orchestrator run
loop via `tracker.kind: jira`; see [jira.md](jira.md).

## 1. Boundary

OpenSymphony uses Linear in two different ways:

- the Rust orchestrator reads Linear through the internal `opensymphony_linear`
  module
- the coding agent reads and writes Linear through the repo-local GraphQL skill
  assets copied into the target repository

Scheduler correctness must never depend on agent-side ticket writes succeeding.

## 2. Orchestrator read adapter

The internal `opensymphony_linear` module is the tracker adapter used by the
daemon when `tracker.kind` is `linear` (the `opensymphony_jira` module fills
the same role for Jira workspaces).

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

- `tracker.kind` must be `linear` (use `jira` for Jira workspaces)
- `tracker.project_slug` stores Linear `Project.slugId`
- `LINEAR_API_KEY` must be available when Linear mode is enabled
- if `LINEAR_CLIENT_ID` and `LINEAR_CLIENT_SECRET` are both present,
  `opensymphony run` mints a Linear OAuth client-credentials access token at
  startup, uses it for the scheduler's Linear tracker client, and exposes it to
  workers as `LINEAR_API_KEY`
- the scheduler's Linear tracker client remains mandatory for `opensymphony run`;
  the gateway task graph reader is optional and, when unavailable, causes only
  the task graph endpoint to return `503`

Local scheduler polling keeps the 5s worker/snapshot tick, but Linear reads are
cadenced separately so a busy local workstation does not burn through the shared
Linear API quota:

- running issue state is refreshed with the lightweight by-ID state query every
  30s
- dispatch discovery uses a lightweight active-issue summary query every 60s
- terminal cleanup reads run at startup and then every 5 minutes
- full-detail active issue reads run at startup, for selected dispatches, and
  then hourly

The lightweight dispatch query returns summary-shaped scheduler data only.
Selected candidates must be reloaded through the project-scoped full-detail
lookup before workspace creation, prompt construction, or worker launch.

When Linear returns a rate-limit error with retry metadata longer than the
client's short retry backoff, the Linear client returns the error immediately
instead of sleeping inside the request. The inline retry boundary is the lower
of `tracker.retry_policy.max_backoff` and 30 seconds; longer reset windows are
handed to the scheduler. The scheduler records one shared Linear cooldown,
skips Linear reads until it expires, and continues draining worker updates and
publishing snapshots.

Important normalization rules:

- `blocked_by` is derived from `inverseRelations` entries whose relation type is
  `blocks`; gateway task graph responses filter these IDs to nodes present in
  the returned project snapshot so clients do not receive dangling graph edges
- `state_kind` is derived from Linear's stable workflow-state `type`; clients and
  caches must not infer categories from mutable display names such as
  "Human Review"
- `branch_name` comes from Linear `Issue.branchName` when present and is carried
  through tracker normalization so run-detail clients can show the same branch
  known to the scheduler
- `pr_url` comes from Linear issue attachments only when the attachment has an
  explicit GitHub `sourceType` and a canonical
  `https://github.com/<owner>/<repo>/pull/<number>` URL; generic URL
  attachments are ignored for Run Detail PR metadata
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

When `opensymphony run` minted a Linear OAuth client-credentials token, spawned
workers receive that value as an environment overlay. The target repo's shell
startup files do not need to be changed for those workers.

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
- COE-401: Web App Entry And Deployment Modes
- COE-407: Browser Transport And Remote Stream Protocols
- COE-419: Hosted Auth Placeholders And Web Parity
- COE-473: Desktop task graph dependency and run detail parity
- COE-486: Harness Interrupt Contract And Run Diagnostics
- COE-487: Desktop Run Detail TUI Parity
- COE-488: Lazy Desktop Launcher Command
- COE-489: OpenHands Agent-Server Interrupt Adapter
- COE-490: Codex App-Server Turn Interrupt Adapter
- COE-491: Desktop Run Detail Action Wiring And Cleanup
- COE-492: Merging Supersedes Human Review Polling
- COE-493: Desktop Operations Integration Hardening
- COE-504: Linear Polling And Rate-Limit Recovery

## Source refs

- COE-254
- COE-263
- COE-264
- COE-267
- COE-268
- COE-270
- COE-277
- COE-401
- COE-407
- COE-419
- COE-473
- COE-486
- COE-487
- COE-488
- COE-489
- COE-490
- COE-491
- COE-492
- COE-493
- COE-504

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
