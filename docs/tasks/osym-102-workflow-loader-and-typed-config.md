---
id: OSYM-102
title: Implement workflow loader and typed config
type: feature
area: workflow-config
priority: P0
estimate: 4d
milestone: M1 Foundation and contracts
parent: OSYM-100
depends_on:
  - OSYM-101
blocks:
  - OSYM-204
  - OSYM-304
  - OSYM-305
project_context:
  - AGENTS.md
  - README.md
  - docs/symphony-spec-alignment.md
  - docs/architecture.md
  - WORKFLOW.example.md
repo_paths:
  - crates/opensymphony-workflow/
  - examples/configs/
  - WORKFLOW.example.md
definition_of_ready:
  - OSYM-101 is merged
  - Template engine choice is agreed
---

# OSYM-102: Implement workflow loader and typed config

## Summary
Implement `WORKFLOW.md` loading, YAML front matter parsing, strict template rendering, env-aware config resolution, and the OpenHands extension namespace required by OpenSymphony.

## Scope
- Parse `WORKFLOW.md` into front matter and Markdown body
- Validate required tracker fields and defaults
- Implement strict prompt rendering with `issue` and `attempt` inputs
- Add `openhands` extension config types without polluting upstream Symphony semantics
- Provide clear error types for invalid config or template usage

## Out of scope
- Loading non-root workflow files unless explicitly configured later
- Loose template rendering that ignores unknown variables

## Deliverables
- Typed workflow/config models
- Template renderer
- Config validation helpers
- Example workflow file updates

## Acceptance criteria
- Unknown template variables fail deterministically
- Default values are applied as documented
- The `openhands` extension config can be read without leaking into non-runtime code

## Test plan
- Unit tests for valid and invalid front matter
- Template rendering tests for first-run and continuation contexts
- Env substitution and path-resolution tests
