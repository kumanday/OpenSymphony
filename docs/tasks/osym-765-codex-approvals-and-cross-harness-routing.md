---
id: OSYM-765
title: Codex Approvals And Cross-Harness Routing
milestone: "M10.3: Codex And Subscription Readiness"
priority: 3
estimate: 8
blockedBy: ["OSYM-763", "OSYM-764", "OSYM-767"]
blocks: ["OSYM-770", "OSYM-771"]
parent: null
---

## Summary

Map Codex approval events into the OpenSymphony approval center and let
`opensymphony run` execute issues with the selected harness and model.

## Scope

### In scope

- Map Codex approval requests to OpenSymphony approval requests.
- Build Codex approval decision requests for `approval/respond`.
- Audit approval decisions with correlation back to the request/run.
- Support explicit harness and model selection through workflow config and
  operator/launcher environment overrides.
- Wire scheduler/runtime dispatch selection for `opensymphony run` so selected
  Codex harness execution uses the Codex worker backend.
- Add route decision audit events.
- Add `opensymphony run --dry-run` route preview.

### Out of scope

- Hosted Codex production pool.
- Automatic cost optimization.
- Additional tracker adapters.

## Deliverables

- Codex approval bridge.
- Approval contract tests.
- Harness/model selection alpha.
- Route decision tests.
- Dry-run route preview.

## Acceptance Criteria

- [ ] Codex approval requests appear in the same approval center contract as OpenHands-supported approvals.
- [ ] Codex approval decision requests serialize to `approval/respond`, and the
      audit record stays correlated with the approval ID and run.
- [ ] `opensymphony run` can select the Codex app-server harness for issue
      execution when the operator/workflow selects it.
- [ ] Selected models are passed to the selected harness where supported:
      OpenHands receives the selected model as its conversation LLM model, and
      Codex receives an explicit `thread/start` and `turn/start` model only when
      one is selected.
- [ ] Codex app-server execution uses the full-automation profile: hook trust
      bypass at server launch plus `approvalPolicy: "never"` and
      `dangerFullAccess` turn sandbox policy validated against the installed
      Codex schema.
- [ ] Dry-runs explain the selected harness, model, and model profile.

## Test Plan

- Run Codex approval bridge tests with fake JSON-RPC notifications.
- Run scheduler/runtime route selection tests that prove `opensymphony run`
  dispatches through the selected harness backend.
- Run route decision tests for selected harness/model, environment overrides,
  and unavailable-harness cases.
- Run Codex lifecycle tests that prove OpenSymphony generates the installed
  app-server schema, validates outbound automation payloads, creates a thread,
  and starts a turn without human approval waits.

## Context

- Source sections: `docs/host-client-implementation_plan.md` P9.7/P9.8.
- Routing should use configured base URL, model string, credential reference, and harness capabilities.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Treat this as alpha behavior with explicit user visibility.
