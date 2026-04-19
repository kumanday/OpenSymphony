# Using The Helper

Use the repo-local helper for every Linear GraphQL call:

```bash
python3 .agents/skills/linear/scripts/linear_graphql.py \
  --query-file .agents/skills/linear/queries/viewer.graphql
```

## Variables

Inline JSON:

```bash
python3 .agents/skills/linear/scripts/linear_graphql.py \
  --query-file .agents/skills/linear/queries/issue_by_key.graphql \
  --variables '{"key":"COE-123"}'
```

From a file:

```bash
python3 .agents/skills/linear/scripts/linear_graphql.py \
  --query-file .agents/skills/linear/queries/project_update_content.graphql \
  --variables-file /tmp/project-vars.json
```

Issue creation or rewrite example:

```bash
python3 .agents/skills/linear/scripts/linear_graphql.py \
  --query-file .agents/skills/linear/queries/issue_create.graphql \
  --variables-file /tmp/issue-create-vars.json
```

## Rules

- `LINEAR_API_KEY` must be set.
- Use exactly one `.graphql` operation per call.
- Prefer the checked-in query files over improvising large inline documents.
- Treat top-level `errors` as failure.
- Keep requested fields narrow.
- For `issue_create.graphql`, pass the full `IssueCreateInput` object inside
  `variables.input`; for `issue_update.graphql`, pass the target issue id plus
  the partial `IssueUpdateInput` object inside `variables.input`.
- For `fileUpload`, send the follow-up `PUT` with `Content-Type` equal to the
  `contentType` you requested, plus every header returned by Linear.
- Use `queries/introspect_mutations.graphql` and
  `queries/introspect_input_shape.graphql` before guessing unfamiliar shapes.
