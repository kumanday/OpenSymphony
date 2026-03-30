# Linear and Tools

## 1. Boundary

Symphony treats the issue tracker as an orchestration input and leaves most ticket writes to the coding agent.

OpenSymphony follows that boundary strictly:

- the orchestrator reads Linear directly through a dedicated adapter
- the agent performs Linear writes through tools, preferably MCP
- scheduler correctness must not depend on ticket writes succeeding

## 2. Linear read adapter responsibilities

The Rust Linear adapter must implement the minimum read surface Symphony requires.

Current repository implementation:

- `opensymphony-orchestrator::Scheduler` now drives every tick from three Linear read paths: active candidates, terminal issues, and by-ID state refresh for anything already tracked locally
- candidate reads decide new dispatches, by-ID refresh releases work that falls out of the configured active states, and terminal reads drive startup cleanup plus terminal reconciliation
- `opensymphony-linear::LinearClient` also exposes `fetch_workpad_comment(issue_id)` for runtime recovery flows that need the latest active `## Agent Harness Workpad` comment without coupling the OpenHands runtime directly to Linear GraphQL details

## 2.1 Candidate issue fetch

Fetch issues that:

- belong to the configured project slug
- are in configured active states
- include enough fields to normalize issue data and eligibility

Implementation note:

- the GraphQL adapter filters by Linear `Project.slugId`, so workflow `tracker.project_slug` should store that stable slug value
- candidate issue polling should exclude archived issues so archiving an active ticket releases it instead of redispatching it on the next poll

## 2.2 State refresh by IDs

Fetch current states for all running issues during reconciliation.

Implementation note:

- by-ID reconciliation should keep the same `project_slug` filter as candidate reads so issues moved out of the tracked project fall out of orchestration instead of staying alive
- by-ID reconciliation should pass `includeArchived: true` so archived terminal issues still surface for cleanup and stale retry-manifest removal

## 2.3 Terminal-state fetch for startup cleanup

Fetch issues in terminal states when sweeping existing workspaces on startup.

Implementation note:

- terminal-state cleanup reads should pass `includeArchived: true` so archived terminal work remains visible during startup cleanup

## 2.4 Normalization

Normalize tracker payloads into a stable issue model with fields such as:

- `id`
- `identifier`
- `url`
- `title`
- `description`
- `priority`
- `state`
- `labels`
- `parent_id`
- `blocked_by`
- `sub_issues`
- `created_at`
- `updated_at`

Keep the orchestrator independent of raw GraphQL response shape.

Implementation note:

- blocker normalization should derive `blocked_by` from `inverseRelations` entries where relation `type == "blocks"`
- hierarchy normalization should derive `parent_id` from `parent.id` and `sub_issues` from `children.nodes`
- `TrackerIssue.state` should remain the workflow-facing state name string consumed by `WORKFLOW.md` and `WORKFLOW.example.md`
- blocker and state-refresh normalization should retain both the state name and the raw Linear `WorkflowState.type` string, while also exposing a normalized `kind`, so terminal blockers remain detectable without losing the tracker's exact type value
- sub-issue normalization only needs the child `id`, `identifier`, and state `name` because hierarchy gating compares child state names against the workflow-configured terminal-state names
- issue normalization should retain the Linear issue URL because `WORKFLOW.md` renders `{{ issue.url }}` under strict template validation
- issue normalization should preserve the raw Linear priority because prompt/UI consumers render `{{ issue.priority }}` directly
- top-level issue pages should request only small initial `labels` and `inverseRelations` slices and page the rest per issue so connection-heavy issue metadata stays complete without blowing past Linear's query cap

## 2.5 Workpad comment lookup

Some runtime recovery paths need the latest active workpad comment for an issue.

Current repository implementation:

- `fetch_workpad_comment(issue_id)` pages `issue.comments`
- it ignores resolved comments
- it selects the latest updated comment whose body contains the `## Agent Harness Workpad` marker
- it returns only `id`, `body`, and `updated_at`, keeping the recovery surface intentionally small

## 3. Candidate sorting and eligibility

The orchestrator should receive normalized issues and apply Symphony sorting and eligibility rules.

Recommended sort order:

1. higher urgency first using raw Linear priority (`1` before `2`; `0` remains unprioritized)
2. leaf issues before parents when both are otherwise dispatchable
3. older creation time first
4. identifier tie-breaker

Implementation note:

- Linear's raw priority scale is urgency-inverted (`1` is most urgent, `0` is unprioritized), so the scheduler should derive its sort key from the raw value instead of rewriting the shared issue model
- parent issues should remain ineligible while any `sub_issues` entry is in a non-terminal state, even if blocker relations are otherwise clear

Eligibility reminders:

- active state only
- `Todo` issues with non-terminal blockers are ineligible
- terminal blockers should not prevent eligibility
- currently-running or claimed issues are not redispatched

## 4. Linear GraphQL client design

## 4.1 Client requirements

- typed request and response models
- explicit pagination support
- rate-limit aware retries where appropriate
- structured error classification
- redaction of tokens in logs

Current adapter contract:

- send GraphQL requests to the configured endpoint with `Authorization: <LINEAR_API_KEY>` because the local MVP uses personal Linear API keys rather than OAuth access tokens
- reject blank `LINEAR_API_KEY` values during client construction so startup misconfiguration fails fast instead of repeated auth failures at poll time
- decode GraphQL error envelopes before falling back to raw HTTP classification because Linear rate limits can arrive as HTTP 400 with GraphQL code `RATELIMITED`, but keep transient HTTP 5xx responses retryable even when the body is a GraphQL error envelope
- prefer Linear's `X-RateLimit-*-Reset` headers when calculating retry delays for rate-limited responses, falling back to `Retry-After` and then local exponential backoff only when no reset window is advertised
- keep transient HTTP 5xx responses on the retryable HTTP-status path even when the body decodes as a GraphQL error envelope
- keep GraphQL query and response structs private to `opensymphony-linear`
- return only normalized domain models to orchestrator-facing callers

## 4.2 Configuration inputs

Required:

- `LINEAR_API_KEY`
- workflow `tracker.kind`
- workflow `tracker.project_slug` (the Linear `Project.slugId` value)
- workflow `tracker.kind`
- workflow `tracker.active_states`
- workflow `tracker.terminal_states`

Implementation note:

- the adapter should reject blank `LINEAR_API_KEY` values at client construction time so missing auth fails fast before the daemon starts polling
- the adapter should reject blank `tracker.project_slug` values at client construction time so candidate/cleanup reads fail fast instead of silently querying `slugId == ""`
- the adapter should reject blank or missing `tracker.active_states` / `tracker.terminal_states` lists at client construction time so workflow misconfiguration fails fast instead of returning empty candidate/cleanup results

Optional:

- per-page size
- request timeout
- retry policy

## 5. MCP write surface

For unattended execution, the agent needs a minimal, stable write surface for Linear.

OpenSymphony should provide a small stdio MCP server rather than coupling writes into the orchestrator.

Current repository note:

- workflow-owned OpenHands MCP stdio server declarations now resolve into the runtime request as `mcp_config.stdio_servers`, so unattended sessions can provision the Linear MCP surface through `openhands.mcp.stdio_servers` instead of only the host tool environment
- `opensymphony doctor` can still resolve workflows that omit `tracker.api_key` when `linear.enabled: false`; it uses a doctor-only placeholder for the omitted fallback token so static/local runtime validation does not require live Linear credentials

## 5.1 MVP tool set

Implemented MVP tools:

- `linear_get_issue`
  - fetch an issue by UUID or identifier such as `COE-267`
- `linear_comment_issue`
  - add a comment to an issue and return the created comment plus the resolved issue snapshot
- `linear_transition_issue`
  - move an issue to a named workflow state for that issue's team
- `linear_link_pr`
  - add a PR URL or related link to the issue via a URL attachment
- `linear_list_project_states`
  - fetch valid team workflow states for safer transitions using either an issue reference or a team key/UUID

Do not start with a giant tool surface.

## 5.2 Why MCP instead of direct orchestrator writes

Benefits:

- agent can decide when and how to comment
- the same tool surface works in local and hosted modes
- better alignment with OpenHands tool model
- smaller orchestrator responsibility set

## 5.3 MCP server process model

Recommended command exposed by `opensymphony-cli`:

```text
opensymphony linear-mcp
```

Input dependencies:

- `LINEAR_API_KEY`
- optional `LINEAR_BASE_URL` override for local testing

Current transport notes:

- the stdio transport uses one JSON-RPC message per line
- the server negotiates MCP protocol versions from `2024-11-05` through `2025-11-25`
- the server advertises only the tool capability and keeps the write surface intentionally narrow

## 6. Tool design guidelines

- prefer narrow, explicit schemas
- avoid free-form giant mutation tools
- prefer a single `issue` argument that accepts either a UUID or an issue identifier
- return normalized issue snapshots after mutation when possible
- surface permission and validation errors clearly
- keep tool outputs concise enough for agent context

## 7. Relationship with repository context

The agent already sees:

- repository files
- `WORKFLOW.md`
- repo-root `AGENTS.md` if present in the target repo
- optional repo-owned `.agents/skills/`
- OpenSymphony-generated `.opensymphony/generated/issue-context.md`

Recommended precedence:

1. repo-owned `WORKFLOW.md`
2. repo-owned `AGENTS.md`
3. repo-owned `.agents/skills/`
4. additive OpenSymphony-generated `.opensymphony/generated/issue-context.md`
5. live tracker lookups through Linear MCP tools

The Linear MCP tools should complement this, not duplicate the whole tracker database.
OpenSymphony-generated files should reference repo-owned policy and latest tracker state without overwriting either one, and example target repos should ignore `.opensymphony/` locally so these artifacts remain workspace-scoped.

## 8. Suggested issue model in Rust

```rust
struct Issue {
    id: String,
    identifier: String,
    url: String,
    title: String,
    description: Option<String>,
    priority: Option<u8>,
    state: String,
    labels: Vec<String>,
    parent_id: Option<String>,
    blocked_by: Vec<IssueBlocker>,
    sub_issues: Vec<IssueRef>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
```

`IssueBlocker` should include enough information to decide whether the blocker is terminal.
`IssueRef` should include enough information to decide whether a child issue is terminal from the poll snapshot alone.

## 9. Error categories

Recommended Linear error categories:

- auth
- rate_limited
- transport
- timeout
- invalid_response
- not_found
- invalid_state_transition
- permission_denied

These categories should be shared by the read adapter and the MCP server where sensible.

## 10. Testing plan

### Read adapter

- pagination
- normalization of missing optional fields
- blocker handling
- active vs terminal state fetch
- retry on transient HTTP failures

### MCP server

- stdio startup
- valid tool registration
- happy-path issue comment
- happy-path state transition
- happy-path link attachment
- invalid state name
- auth failure
- concise error surfaces
- MCP handshake and line-delimited stdio framing

### End-to-end

- worker can read issue via orchestrator
- agent can comment or transition via MCP during a live run
- orchestrator remains correct even if MCP writes fail

Current repository coverage:

- `crates/opensymphony-linear-mcp/src/server.rs` locks in the documented tool registration and version negotiation rules
- `crates/opensymphony-cli/tests/linear_mcp.rs` drives `opensymphony linear-mcp` through initialize, `tools/list`, `linear_get_issue`, `linear_comment_issue`, `linear_transition_issue`, `linear_link_pr`, and `linear_list_project_states` against a local fake GraphQL server

## 11. Future hosted-mode notes

The same Linear MCP server can run:

- locally as a stdio process launched by OpenSymphony
- centrally as a network-accessible MCP service later if needed

Start with stdio for simplicity and local reliability.
