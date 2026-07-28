---
id: OSYM-885
title: Canonical Repository Binding And Task Propagation
milestone: "M12.95: Multi-Repository Foundations"
priority: 1
estimate: 8
blockedBy: ["OSYM-884"]
blocks: ["OSYM-886", "OSYM-887", "OSYM-889"]
areas:
  - linear
  - orchestrator
  - planning
parent: null
---

## Summary

Give every terminal implementation task one immutable canonical repository
binding while keeping parent tasks repository-neutral and project associations
non-routing.

## Scope

### In scope

- Add a provider-qualified canonical repository ID and credential-free safe
  remote fingerprint with rename/transfer-safe provider identity when available.
- Parse exactly one managed `repo:<alias>` binding for terminal children in
  strict mode and resolve it against the task's Linear project associations.
- Reject missing, unknown, multiple, disallowed, parent, and out-of-project-set
  bindings as distinct durable blocked states.
- Keep parent tasks free of repository labels and execution-repository binding.
- Propagate repository aliases through task-package validation, Linear
  conversion, planner artifacts, normalized tracker issues, and scheduler
  candidate state without storing remotes or workspace paths in Linear.
- Persist canonical identity, config generation, and inventory generation before
  claiming a task.
- Treat a binding mutation during a run as controlled supersession; late events
  from the old generation must not affect the replacement.
- Preserve legacy unlabelled dispatch only in `legacy_single` mode.

### Out of scope

- Checkout creation and repository-local instruction loading.
- Parent descendant repository derivation.
- Supporting task-level target-branch overrides.

## Deliverables

- Canonical repository identity, safe fingerprint, and typed binding outcomes.
- Task-package and converter validation for managed repository aliases.
- Linear normalization and scheduler propagation of binding state.
- Supersession and stale-event guards.
- Routing, converter, and scheduler regression tests.

## Acceptance Criteria

- [ ] One valid terminal-child alias resolves to one canonical repository ID and
      is recorded before claim.
- [ ] Missing, unknown, multiple, disallowed, parent, and out-of-scope bindings
      produce distinguishable blocked states and create no workspace.
- [ ] A parent with descendants in several repositories has no `repo:` label and
      no execution-repository field.
- [ ] A Linear project association constrains allowed aliases but never chooses a
      default repository.
- [ ] Task-package validation and Linear conversion preserve exactly one managed
      binding while leaving unrelated labels unchanged.
- [ ] Changing a claimed task's binding stops and supersedes the old generation
      without accepting its late worker events.
- [ ] Legacy mode continues to accept unlabelled tasks.

## Test Plan

- Add table-driven alias and association tests for every typed outcome.
- Add converter tests for one binding, duplicate bindings, parent rejection, and
  unmanaged-label preservation.
- Add scheduler tests for claim immutability, supersession, and late events.
- Run focused Linear, planning, domain, and orchestrator suites plus
  `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 4, 5, 7, and 9.
- Inspect `crates/opensymphony-domain/src/{issue,identifiers,state_machine}.rs`,
  `crates/opensymphony-linear/src/normalize.rs`,
  `crates/opensymphony-orchestrator/src/{selection,scheduler}.rs`, and
  `.agents/skills/convert-tasks-to-linear/scripts/convert_tasks_to_linear.py`.
- Canonical IDs are durable identity; aliases and `owner/repository` coordinates
  are human-facing locators.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Do not add a “leaf repository” field to project-set or Linear-project config.
The exactly-one rule belongs only to terminal task metadata.
