# MCP Capabilities

OpenSymphony's stable MVP Linear surface is the stdio MCP server launched as:

```text
opensymphony linear-mcp
```

Use MCP first when the session exposes these tools and the requested operation
fits one of them:

- Verified from OpenSymphony's `tools/list`, the current MCP surface exposes
  exactly five Linear tools.
- `linear_get_issue`
  - Fetch an issue by UUID or identifier such as `COE-267`.
- `linear_comment_issue`
  - Add a new comment to an issue.
- `linear_transition_issue`
  - Move an issue to a named workflow state.
- `linear_link_pr`
  - Attach a PR URL or related URL to the issue.
  - The current implementation uses Linear's generic `attachmentLinkURL`
    mutation rather than the GitHub-specific `attachmentLinkGitHubPR`
    mutation.
- `linear_list_project_states`
  - Despite the legacy name, this returns team workflow states, not Linear
    project status objects.
  - Accept either an `issue` selector or a `team` key/UUID and fetch valid
    workflow states for safer transitions.

Prefer MCP for:

- reading an issue snapshot
- creating a workpad comment
- transitioning an issue
- attaching a PR link
- resolving valid state names before a transition

Do not force MCP for operations it does not cover today. Verified GraphQL-only
gaps in the current OpenSymphony surface include:

- editing an existing comment via `commentUpdate`
- file uploads via `fileUpload`
- issue relation mutations such as `issueRelationCreate`
- GitHub-native PR attachments via `attachmentLinkGitHubPR`
- project overview/content reads and updates through `Project.content` and
  `projectUpdate`
- project status mutations such as `projectStatusCreate` and
  `projectStatusUpdate`
- schema introspection for unfamiliar Linear objects or mutations

For those operations, switch to `linear_graphql` when available or use the raw
GraphQL fallback references.

Choose transport by capability instead of tool name:

- Need to post a fresh workpad comment: use MCP.
- Need to update an existing workpad comment: use GraphQL.
- Need to attach a plain URL or lightweight PR link: MCP is fine.
- Need GitHub-specific attachment semantics: use GraphQL.
- Need project overview/content or project status changes: use GraphQL.
