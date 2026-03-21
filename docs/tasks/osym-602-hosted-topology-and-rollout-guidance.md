---
id: OSYM-602
title: Document hosted topology and rollout guidance
type: feature
area: hosted-follow-on
priority: P2
estimate: 3d
milestone: M6 Hosted deployment follow-on
parent: OSYM-600
depends_on:
  - OSYM-503
  - OSYM-601
project_context:
  - AGENTS.md
  - README.md
  - docs/deployment-modes.md
  - docs/testing-and-operations.md
repo_paths:
  - docs/
  - scripts/
definition_of_ready:
  - OSYM-503 and OSYM-601 are merged
  - Hosted architecture decisions are reviewed
---

# OSYM-602: Document hosted topology and rollout guidance

## Summary
Document the first supported hosted topology for OpenSymphony, including what changes operationally relative to the local MVP and what remains the same architecturally.

## Scope
- Document remote daemon-to-server topology choices
- Document security posture differences from local mode
- Document workspace isolation guidance and rollout sequencing
- Document migration from local MVP to organization-managed deployment

## Out of scope
- Production Terraform, Helm, or autoscaling implementation
- SRE runbooks beyond initial guidance

## Deliverables
- Hosted deployment guidance doc updates
- Hosted rollout checklist tied to the existing transport config surface
- Rollout checklist
- Migration notes from local to hosted mode

## Acceptance criteria
- A technical lead can understand how to move from local MVP to hosted mode without revisiting core architecture assumptions
- The docs clearly distinguish trusted local mode from hosted security requirements
- The docs clearly distinguish first hosted pilot guidance from later fleet automation work

## Test plan
- Docs review and walkthrough with the implemented remote mode
- Walk through the migration from `local_server.enabled=true` to `false` and the related auth settings
