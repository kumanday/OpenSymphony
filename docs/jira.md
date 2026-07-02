# Jira Tracker

OpenSymphony can poll Jira instead of Linear for the orchestrator run loop.
Select it with `tracker.kind: jira` in the target repository's `WORKFLOW.md`
front matter.

## 1. Configuration

```yaml
tracker:
  kind: jira
  endpoint: https://acme.atlassian.net
  project_slug: OSYM
  active_states:
    - To Do
    - In Progress
  terminal_states:
    - Done
```

- `tracker.endpoint` is required and stores the Jira site base URL
  (`https://<site>.atlassian.net` for Jira Cloud, or the base URL of a
  self-hosted Jira Data Center instance). There is no default endpoint.
- `tracker.project_slug` stores the Jira project key (the `OSYM` in
  `OSYM-123`), not a numeric project id.
- `tracker.active_states` and `tracker.terminal_states` use Jira status
  *names* exactly as they appear in the project's workflow.

## 2. Credentials

| Source | Field / Env Var | Notes |
|--------|-----------------|-------|
| API token | `tracker.api_key` or `JIRA_API_TOKEN` | Required |
| Account email | `tracker.auth_email` or `JIRA_EMAIL` | Optional |

When an email is configured, the client authenticates with HTTP basic auth
(`email:token`), which is what Jira Cloud API tokens require. Without an
email, the token is sent as a bearer token, which matches Jira Data Center
personal access tokens.

Both `tracker.api_key` and `tracker.auth_email` support `${VAR}` environment
expansion, mirroring the Linear tracker fields.

## 3. API surface and normalization

The internal `opensymphony_jira` module talks to the Jira Cloud REST API v3:

- `POST /rest/api/3/search/jql` for candidate, terminal, and state-refresh
  reads (paged with `nextPageToken`)
- `GET /rest/api/3/issue/{key}` for identifier lookups
- `GET /rest/api/3/issue/{id}/comment` for Agent Harness Workpad comments

Payloads normalize into the same tracker-neutral domain models Linear uses:

| Jira | Domain |
|------|--------|
| issue `id` | `TrackerIssue.id` |
| issue key (`OSYM-123`) | `TrackerIssue.identifier` |
| status name | `TrackerIssue.state` |
| status category (`new`/`indeterminate`/`done`) | `TrackerIssueStateKind` (`Unstarted`/`Started`/`Completed`) |
| priority (`Highest`..`Lowest`, or numeric id) | `TrackerIssue.priority` (`1`..`4`; unrecognized schemes map to none) |
| `parent` / `subtasks` | `parent` / `sub_issues` |
| `issuelinks` with inward `is blocked by` | `blocked_by` |
| first `fixVersions` entry | `project_milestone` |
| project `id` / key / name | `project_id` / `project_slug` / `project_name` |

Rich-text fields (descriptions and comment bodies) arrive as Atlassian
Document Format and are flattened to markdown-ish plain text, preserving
`##`-style headings so the `## Agent Harness Workpad` comment marker keeps
working.

Rate limiting honors `Retry-After` and Jira's `X-RateLimit-Reset` headers,
with the same retry policy semantics as the Linear client.

## 4. Current scope

Supported with `tracker.kind: jira`:

- the scheduler tracker backend (candidate polling, state refresh, terminal
  cleanup, identifier lookups)
- workpad comment rehydration for issue sessions
- gateway task graph reads

Still Linear-only:

- planning publish artifacts (`convert-tasks-to-linear`) and gateway task
  graph mutations
- `opensymphony memory` tracker sources (the memory commands fail with a
  clear error when the workflow tracker is Jira)
- Jira does not expose branch names or PR attachments through the plain REST
  issue payload, so `branch_name` and `pr_url` are always empty; PR linkage
  relies on the worker-side flow instead.
