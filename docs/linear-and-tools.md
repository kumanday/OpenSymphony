# Linear and Tools

## 1. Boundary

Symphony treats the issue tracker as an orchestration input and leaves most ticket writes to the coding agent.

OpenSymphony follows that boundary strictly:

- the orchestrator reads Linear directly through a dedicated adapter
- the agent performs Linear writes through tools, preferably MCP
- scheduler correctness must not depend on ticket writes succeeding

## 2. Linear read adapter responsibilities

The Rust Linear adapter must implement the minimum read surface Symphony requires.

## 2.1 Candidate issue fetch

Fetch issues that:

- belong to the configured project slug
- are in configured active states
- include enough fields to normalize issue data and eligibility

Implementation note:

- the GraphQL adapter filters by Linear `Project.slugId`, so workflow `tracker.project_slug` should store that stable slug value
- candidate and terminal-state `issues(...)` reads should pass `includeArchived: true` so cleanup can still observe archived terminal work

## 2.2 State refresh by IDs

Fetch current states for all running issues during reconciliation.

Implementation note:

- by-ID reconciliation should keep the same `project_slug` filter as candidate reads so issues moved out of the tracked project fall out of orchestration instead of staying alive
- by-ID reconciliation should exclude archived issues so operators can archive live work to release it from orchestration immediately

## 2.3 Terminal-state fetch for startup cleanup

Fetch issues in terminal states when sweeping existing workspaces on startup.

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
- `blocked_by`
- `created_at`
- `updated_at`

Keep the orchestrator independent of raw GraphQL response shape.

Implementation note:

- blocker normalization should derive `blocked_by` from `inverseRelations` entries where relation `type == "blocks"`
- `TrackerIssue.state` should remain the workflow-facing state name string consumed by `WORKFLOW.md` and `WORKFLOW.example.md`
- blocker and state-refresh normalization should retain both the state name and the raw Linear `WorkflowState.type` string, while also exposing a normalized `kind`, so terminal blockers remain detectable without losing the tracker's exact type value
- issue normalization should retain the Linear issue URL because `WORKFLOW.md` renders `{{ issue.url }}` under strict template validation
- issue normalization should preserve the raw Linear priority because prompt/UI consumers render `{{ issue.priority }}` directly
- top-level issue pages should request only a small initial `inverseRelations` slice and page the rest per issue so nested connection complexity stays under Linear's query cap

## 3. Candidate sorting and eligibility

The orchestrator should receive normalized issues and apply Symphony sorting and eligibility rules.

Recommended sort order:

1. higher urgency first using raw Linear priority (`1` before `2`; `0` remains unprioritized)
2. older creation time first
3. identifier tie-breaker

Implementation note:

- Linear's raw priority scale is urgency-inverted (`1` is most urgent, `0` is unprioritized), so the scheduler should derive its sort key from the raw value instead of rewriting the shared issue model

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
- decode GraphQL error envelopes before falling back to raw HTTP classification because Linear rate limits can arrive as HTTP 400 with GraphQL code `RATELIMITED`
- prefer Linear's `X-RateLimit-*-Reset` headers when calculating retry delays for rate-limited responses, falling back to `Retry-After` and then local exponential backoff only when no reset window is advertised
- keep GraphQL query and response structs private to `opensymphony-linear`
- return only normalized domain models to orchestrator-facing callers

## 4.2 Configuration inputs

Required:

- `LINEAR_API_KEY`
- workflow `tracker.project_slug` (the Linear `Project.slugId` value)
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

## 5.1 MVP tool set

Recommended tools:

- `linear_get_issue`
  - fetch issue by identifier or ID
- `linear_comment_issue`
  - add a comment to an issue
- `linear_transition_issue`
  - move issue to a named state
- `linear_link_pr`
  - add a PR URL or related link to the issue
- `linear_list_project_states`
  - fetch valid state names for safer transitions

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
opensymphony linear-mcp --stdio
```

Input dependencies:

- `LINEAR_API_KEY`
- optional config file for org or project defaults

## 6. Tool design guidelines

- prefer narrow, explicit schemas
- avoid free-form giant mutation tools
- return normalized issue snapshots after mutation when possible
- surface permission and validation errors clearly
- keep tool outputs concise enough for agent context

## 7. Relationship with repository context

The agent already sees:

- repository files
- `WORKFLOW.md`
- repo-root `AGENTS.md` if present in the target repo
- OpenSymphony-generated `.opensymphony/generated/issue-context.md`

The Linear MCP tools should complement this, not duplicate the whole tracker database.

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
    blocked_by: Vec<IssueBlocker>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
```

`IssueBlocker` should include enough information to decide whether the blocker is terminal.

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
- invalid state name
- auth failure
- concise error surfaces

### End-to-end

- worker can read issue via orchestrator
- agent can comment or transition via MCP during a live run
- orchestrator remains correct even if MCP writes fail

## 11. Future hosted-mode notes

The same Linear MCP server can run:

- locally as a stdio process launched by OpenSymphony
- centrally as a network-accessible MCP service later if needed

Start with stdio for simplicity and local reliability.
