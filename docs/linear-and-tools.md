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

- belong to the configured project slug value, which maps to Linear's `Project.slugId`
- are in configured active states
- include enough fields to normalize issue data and eligibility

Use Linear relation fields, not deprecated shortcut fields, when deriving blockers. In practice this means mapping inverse issue relations of type `blocks` into `blocked_by`.

## 2.2 State refresh by IDs

Fetch current states for all running issues during reconciliation.

## 2.3 Terminal-state fetch for startup cleanup

Fetch issues in terminal states when sweeping existing workspaces on startup.

## 2.4 Normalization

Normalize tracker payloads into a stable issue model with fields such as:

- `id`
- `identifier`
- `title`
- `description`
- `priority`
- `state`
- `labels`
- `blocked_by`
- `created_at`
- `updated_at`

Keep the orchestrator independent of raw GraphQL response shape.

## 3. Candidate sorting and eligibility

The orchestrator should receive normalized issues and apply Symphony sorting and eligibility rules.

Recommended sort order:

1. higher priority first
2. older creation time first
3. identifier tie-breaker

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
- transient HTTP status classification even when the response body is not valid JSON
- redaction of tokens in logs

## 4.2 Configuration inputs

Required:

- `LINEAR_API_KEY`
- workflow `tracker.project_slug` mapped to Linear `Project.slugId`
- workflow `tracker.active_states`
- workflow `tracker.terminal_states`

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
- optional `OPENSYMPHONY_LINEAR_API_URL` override for non-default Linear GraphQL endpoints
- optional config file for org or project defaults

Test-only fallback:

- `OPENSYMPHONY_LINEAR_FIXTURE`
  - only for fixture-backed local tests
  - do not use as the default unattended write backend

## 6. Tool design guidelines

- prefer narrow, explicit schemas
- avoid free-form giant mutation tools
- return normalized issue snapshots after mutation when possible
- surface permission and validation errors clearly
- return JSON-RPC/MCP error responses for failed tool calls without terminating the stdio server
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
    title: String,
    description: Option<String>,
    priority: Option<i64>,
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
For retryable failures, status-based classification must win even when the response body is plain text or HTML. In practice this means `429` stays `rate_limited`, gateway timeouts stay `timeout`, and `5xx` responses stay `transport` instead of degrading into `invalid_response`.

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
- backend/tool failure returns a JSON-RPC error and the server remains available for later requests
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
