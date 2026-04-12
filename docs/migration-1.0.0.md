# OpenSymphony 1.0.0 Migration Guide

OpenSymphony 1.0.0 is the breaking release that removes the old agent-side
Linear bridge layer and standardizes AI PR review configuration.

## Breaking changes

### 1. `openhands.mcp` was removed

Older target repos may still contain workflow config like:

```yaml
openhands:
  mcp:
    ...
```

That block is no longer supported.

What to do instead:

- remove the `openhands.mcp` section
- ensure `LINEAR_API_KEY` is available in the target repo environment
- use the repo-local `.agents/skills/linear/` helper, query files, and
  references copied by `opensymphony init`

The workflow loader now fails fast with a migration error when it sees the
removed field.

### 2. The old bridge CLI entrypoint is gone

Removed command:

```bash
opensymphony linear-mcp
```

There is no replacement bridge process. Agent-side Linear operations now run
through checked-in GraphQL assets inside the target repo.

### 3. AI review secrets are provider-agnostic

Old secret naming tied the workflow too closely to one inference provider.

Use this secret name now:

- `AI_REVIEW_API_KEY`

Supported variables:

- `AI_REVIEW_PROVIDER_KIND`
- `AI_REVIEW_MODEL_ID`
- `AI_REVIEW_BASE_URL`
- `AI_REVIEW_STYLE`
- `AI_REVIEW_REQUIRE_EVIDENCE`

`FIREWORKS_API_KEY` is no longer part of the supported configuration.

## Upgrade checklist

1. Upgrade to OpenSymphony 1.0.0.
2. Re-run `opensymphony init` in target repos that still carry older generated
   assets.
3. Remove any `openhands.mcp` config from repo workflows.
4. Set `LINEAR_API_KEY` for target repos that need Linear access.
5. Update GitHub Actions secrets and variables for AI review:
   - secret: `AI_REVIEW_API_KEY`
   - variables: provider/model/base URL/style/evidence
6. Run:

```bash
cargo test --workspace
cargo test -p opensymphony-cli --test init
```

## Smoke checks

After migration:

- `opensymphony --help` should not list the removed bridge command
- `opensymphony init` should copy the full `.agents/skills/linear/` tree
- this helper should work from the target repo:

```bash
python3 .agents/skills/linear/scripts/linear_graphql.py \
  --query-file .agents/skills/linear/queries/viewer.graphql
```
