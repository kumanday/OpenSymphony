# Issue And Comment Operations

Use these checked-in query files for common issue-side work:

- Create an issue or sub-issue: `queries/issue_create.graphql`
- Update an issue title, body, parent, project, or state metadata: `queries/issue_update.graphql`
- Read an issue by key: `queries/issue_by_key.graphql`
- Read a full issue snapshot by id: `queries/issue_details.graphql`
- Read issue comments by id: `queries/issue_comments.graphql`
- Resolve valid workflow states for an issue: `queries/issue_team_states.graphql`
- Create a comment: `queries/comment_create.graphql`
- Update a comment: `queries/comment_update.graphql`
- Move an issue to another state: `queries/issue_move_to_state.graphql`
- Create a blocker/related relation: `queries/issue_relation_create.graphql`
- Attach a GitHub PR with native semantics: `queries/attachment_link_github_pr.graphql`
- Attach a generic URL: `queries/attachment_link_url.graphql`

## Notes

- `issue_create.graphql` accepts a full `IssueCreateInput`; use it for parent
  issues, follow-up issues, and sub-issues once the required `teamId` and any
  optional `projectId`, `stateId`, or `parentId` values are known.
- `issue_update.graphql` expects the internal issue id plus a partial
  `IssueUpdateInput`; use it for post-create rewrite passes once real Linear
  issue identifiers and URLs are known.
- Prefer `attachmentLinkGitHubPR` for real PR links.
- Use `attachmentLinkURL` only when the target is not a GitHub PR or the
  generic URL behavior is explicitly desired.
- `issue_move_to_state.graphql` expects a `stateId`; resolve it first with
  `issue_team_states.graphql`.
- `issue_comments.graphql` returns active comments with `resolvedAt`, `updatedAt`,
  and author identity so agents can sort recent operator feedback and ignore
  resolved workpad/comment threads when needed.
