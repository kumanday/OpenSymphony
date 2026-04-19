---
name: linear
description: |
  Use repo-local GraphQL helpers to read and write Linear through
  `LINEAR_API_KEY`.
---

# Linear

Use this skill whenever the agent needs to read or write Linear state from an
OpenSymphony-managed repository.

## Required auth

- `LINEAR_API_KEY` must be present in the environment.
- If it is missing, treat Linear access as a real blocker.
- `tracker.project_slug` stores Linear `Project.slugId`, not a display name.

## Primary path

Run the repo-local helper:

```bash
python3 .agents/skills/linear/scripts/linear_graphql.py \
  --query-file .agents/skills/linear/queries/viewer.graphql
```

Pass variables as inline JSON or a JSON file:

```bash
python3 .agents/skills/linear/scripts/linear_graphql.py \
  --query-file .agents/skills/linear/queries/issue_by_key.graphql \
  --variables '{"key":"COE-123"}'
```

## References

- Start with [references/using-the-helper.md](references/using-the-helper.md).
- For issue creation, updates, comments, relations, and PR-link work, open
  [references/issue-and-comment-operations.md](references/issue-and-comment-operations.md).
- For project overview/status updates, uploads, and introspection, open
  [references/project-and-advanced-operations.md](references/project-and-advanced-operations.md).

## Rules

- This repo uses GraphQL-only Linear access through the checked-in helper.
- Prefer query files under `.agents/skills/linear/queries/` over ad hoc inline
  GraphQL strings.
- Keep exactly one GraphQL operation per request.
- Use variables instead of string interpolation.
- Treat a top-level `errors` array as a failed Linear operation even if the
  HTTP request succeeds.
- Keep requested fields narrow and task-specific.
- When an unfamiliar mutation or input shape is needed, start with the
  introspection query files before guessing.
- Use the reference docs for exact variable shapes and example commands instead
  of copying large GraphQL documents inline.

## Common workflows

- Create an issue or sub-issue:
  - `.agents/skills/linear/queries/issue_create.graphql`
- Update an issue title, body, or metadata:
  - `.agents/skills/linear/queries/issue_update.graphql`
- Create a comment:
  - `.agents/skills/linear/queries/comment_create.graphql`
- Edit a comment:
  - `.agents/skills/linear/queries/comment_update.graphql`
- Move an issue to a new state:
  - `.agents/skills/linear/queries/issue_team_states.graphql`
  - `.agents/skills/linear/queries/issue_move_to_state.graphql`
- Attach a GitHub PR:
  - `.agents/skills/linear/queries/attachment_link_github_pr.graphql`
- Attach a plain URL:
  - `.agents/skills/linear/queries/attachment_link_url.graphql`
- Create an issue relation:
  - `.agents/skills/linear/queries/issue_relation_create.graphql`
- Update project overview/content:
  - `.agents/skills/linear/queries/project_by_slug.graphql`
  - `.agents/skills/linear/queries/project_update_content.graphql`
- Create or update project status:
  - `.agents/skills/linear/queries/project_status_create.graphql`
  - `.agents/skills/linear/queries/project_status_update.graphql`
  - `.agents/skills/linear/queries/project_set_status.graphql`
- Upload a file for a comment:
  - `.agents/skills/linear/queries/file_upload.graphql`
  - upload the bytes to the returned `uploadUrl` with `Content-Type` matching
    the requested `contentType` plus every header returned by Linear
  - then create or update the comment with the returned `assetUrl`
- Inspect the schema:
  - `.agents/skills/linear/queries/introspect_mutations.graphql`
  - `.agents/skills/linear/queries/introspect_input_shape.graphql`
