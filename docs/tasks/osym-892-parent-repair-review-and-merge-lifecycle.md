---
id: OSYM-892
title: Parent Repair Review And Merge Lifecycle
milestone: "M12.96: Parent Integration Lifecycle"
priority: 2
estimate: 13
blockedBy: ["OSYM-891"]
blocks: ["OSYM-893"]
areas:
  - orchestrator
  - provider
  - review
  - workspace-lifecycle
parent: null
---

## Summary

Turn integration defects into durable, repository-specific repair attempts that
create new branches and pull requests, complete configured review, merge
idempotently, and refresh before final verification.

## Scope

### In scope

- Create one immutable repair-attempt entity per parent, canonical repository,
  and attempt number.
- Start a fresh branch from the verified target commit in the affected parent
  integration checkout and acquire a repair lease.
- Reload and hash that repository's current instructions before implementation.
- Keep credentials, branch naming, review requirements, merge method, and target
  branch centrally owned.
- Commit and push through typed credential paths and generation-bound checkout
  handles.
- Create or find the pull request with stable idempotency metadata.
- Reconcile checks, automated and human review, requested changes, external
  closure, force-push, conflicts, and provider outages into durable states.
- Keep requested-change work in the same repair-attempt and pull-request history.
- Merge only when the configured review profile is satisfied.
- Record the provider merge-result commit, verify reachability from the refreshed
  target, and never require replaced feature commits after squash/rebase merge.
- Support several attempts in one repository or across several repositories
  without overwriting prior evidence.
- Reconcile provider truth before repeating a side effect after restart.
- Refresh every affected integration checkout before final verification.
- Keep the domain provider-neutral while implementing GitHub first.

### Out of scope

- Additional source-control providers.
- Bypassing repository review policy for parent fixes.
- Combining several repositories into one branch or pull request.

## Deliverables

- Durable repair-attempt and provider-operation records.
- GitHub-backed branch, push, pull-request, review, check, and merge adapter path.
- Requested-change and multi-attempt controller behavior.
- Provider reconciliation and idempotency tests.
- Repair lifecycle and review-policy documentation.

## Acceptance Criteria

- [ ] A defect isolated to one repository creates one branch and one pull request
      only in that repository.
- [ ] The attempt begins at the recorded target commit and uses the repository's
      current instruction hash.
- [ ] Crash after branch creation, push, pull-request creation, review request,
      approval, or merge creates no duplicate side effect.
- [ ] Requested changes preserve the same attempt and pull-request history.
- [ ] Failed checks, rejection, outage, external closure, force-push, and merge
      conflict produce precise resumable states.
- [ ] Squash and rebase merges complete through provider merge-result
      reachability rather than feature-branch ancestry.
- [ ] Several attempts remain separately auditable.
- [ ] Final verification runs only after every affected repository refreshes from
      the recorded post-merge target.

## Test Plan

- Extend the fake provider with deterministic branch/PR/check/review/merge state
  and call counters.
- Inject crashes around every external side effect and verify search-before-
  create/idempotency behavior.
- Add requested-change, external-close, force-push, merge-conflict, outage,
  squash, rebase, and multi-repository repair fixtures.
- Run focused provider, orchestrator, workspace, and review tests plus
  `cargo clippy-system-duckdb` and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 13.5, 14, 15, 16,
  and 18.1.
- Reuse existing review-profile and AI PR review behavior; do not let a
  repository-local instruction weaken central review requirements.
- Provider state is authoritative for PR, check, review, and merge facts.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

A repair attempt is durable history, not one mutable parent field. Preserve
earlier attempts even when a later attempt supersedes their implementation.
