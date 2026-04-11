# Repository Layout

This document records the intended crate and directory ownership for the
OpenSymphony implementation repo.

## 1. Top-level layout

```text
OpenSymphony/
  AGENTS.md
  README.md
  WORKFLOW.example.md
  Cargo.toml
  crates/
  docs/
  examples/
  scripts/
  tools/
  .github/
```

## 2. Crate boundaries

### `opensymphony-domain`

- shared domain types
- scheduler state and transitions
- snapshot models

### `opensymphony-workflow`

- `WORKFLOW.md` loading
- typed front-matter resolution
- strict prompt rendering
- environment and path resolution
- migration errors for removed workflow fields

### `opensymphony-workspace`

- workspace path resolution
- containment and sanitization
- lifecycle hooks
- issue and conversation manifests

### `opensymphony-linear`

- Linear GraphQL read adapter
- pagination and normalization
- tracker reconciliation helpers

### `opensymphony-openhands`

- local server supervision
- REST client
- WebSocket event stream
- issue session runner

### `opensymphony-orchestrator`

- scheduler loop
- retry queue
- reconciliation
- worker supervision

### `opensymphony-control`

- control-plane HTTP API
- snapshot publication

### `opensymphony-cli`

- `init`
- `run`
- `debug`
- `daemon`
- `tui`
- `doctor`
- `rehydrate`

### `opensymphony-tui`

- FrankenTUI operator UI

### `opensymphony-testkit`

- fake OpenHands helpers
- fake Linear fixtures
- contract-test utilities

## 3. Shared non-crate assets

### `tools/openhands-server/`

Owns the pinned local OpenHands package and launch scripts.

### `examples/`

Holds sample configs and target-repo fixtures.

### `docs/`

Owns design, operations, and migration documentation.

### `.agents/skills/` in the template repo

Owns target-repo agent guidance. The most important Linear assets now live in
the template skill tree instead of a separate bridge crate:

- `SKILL.md`
- `scripts/linear_graphql.py`
- `queries/*.graphql`
- `references/*.md`

## 4. Init propagation rule

`opensymphony init` must copy `.agents/skills/` recursively so that target repos
receive the complete skill payload, including helper scripts and query assets.

That rule is now part of the supported public behavior.

## 5. Versioning note

OpenSymphony `1.0.0` removed the old agent-side Linear bridge layer. The crate
layout above is the post-removal structure and should stay free of dead bridge
code.
