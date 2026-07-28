---
id: OSYM-884
title: Central Multi-Repository Config And Safe Migration
milestone: "M12.95: Multi-Repository Foundations"
priority: 1
estimate: 13
blockedBy: []
blocks: ["OSYM-885"]
areas:
  - configuration
  - migration
  - workflow
parent: null
---

## Summary

Replace current-directory and target-repository orchestration discovery with one
typed, instance-owned configuration model while preserving an explicit,
unchanged legacy single-repository mode.

## Scope

### In scope

- Load `--config <path>` or the default `~/.opensymphony/config.yaml` before
  consulting any repository checkout.
- Add typed instance, routing mode, tracker profile, project set, Linear
  project, repository inventory, credential reference, review profile,
  workspace, scheduler, integration, and memory-catalog sections.
- Keep project-to-repository associations as allowed sets only; do not add a
  project-level default or terminal-repository field.
- Resolve optional project-set integration instructions relative to the central
  config, reject inventory-checkout placement, and persist their content hash.
- Validate all references, unique aliases, non-overlapping roots, contained
  instruction paths, credential-free remotes, strict unknown fields, and
  non-serializable resolved secrets.
- Preserve `legacy_single` as an explicit config variant that dispatches
  unlabelled existing tasks without using an empty inventory as a sentinel.
- Add read-only migration preflight, staged atomic apply, activation marker,
  recoverable backup, and rollback checks.
- Move recognized orchestrator-owned `WORKFLOW.md` front matter into central
  config while preserving repository implementation instructions.
- Keep strict multi-repository routing disabled until later release gates pass.

### Out of scope

- Resolving task repository labels.
- Creating or verifying repository checkouts.
- Enabling parent integration or hosted sandboxing.

## Deliverables

- Typed central config loader, resolver, validation errors, and generation hash.
- Explicit strict and legacy routing variants.
- Migration preflight/apply/rollback commands and reports.
- Config-selection, validation, secret-redaction, and compatibility tests.
- Updated configuration, architecture, migration, and operations documentation.

## Acceptance Criteria

- [ ] `opensymphony run --config <path>` behaves independently of the current
      directory and records one config generation for the run.
- [ ] The default user config and two explicitly selected instance configs use
      non-overlapping state and workspace roots.
- [ ] A project may associate with several repositories and a repository with
      several projects without selecting a default execution repository.
- [ ] Invalid references, duplicate aliases, overlapping roots, credential-
      bearing URLs, checkout-local integration instructions, and unknown strict
      fields fail before tracker polling or workspace creation.
- [ ] Legacy mode continues to dispatch existing unlabelled tasks with the
      current single-repository behavior.
- [ ] Preflight performs no writes, apply is atomic and repeatable, and rollback
      restores the prior runnable generation when no active strict run blocks it.
- [ ] No resolved secret is present in serialized config, logs, errors, or
      migration reports.

## Test Plan

- Add loader and resolver tests covering selection order, path expansion,
  reference resolution, alias collisions, root overlap, strict unknown fields,
  and secret canaries.
- Add fixture migrations for repo-local config and `WORKFLOW.md` front matter,
  including interrupted apply and repeat apply.
- Run focused workflow and CLI config tests, `cargo fmt --check`,
  `cargo clippy-system-duckdb`, and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 5 through 8, 17,
  20, and 23 before implementation.
- Inspect `crates/opensymphony-cli/src/orchestrator_run/config.rs` and
  `crates/opensymphony-workflow/src/{loader,model,resolve}.rs`.
- Preserve current runtime behavior documented in `docs/configuration.md` until
  strict mode is explicitly selected.
- Repository-local `AGENTS.md` and the body of legacy `WORKFLOW.md` remain
  implementation guidance, not central orchestration configuration.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Do not introduce nested config profiles or a second repository-set abstraction.
Separate config files and existing project associations cover the required
boundaries.
