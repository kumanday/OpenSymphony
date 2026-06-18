# Implementation Plan

This document is the planning index for OpenSymphony. It is designed to be directly convertible into Linear issues.

## 1. Planning principles

- preserve Symphony semantics first
- keep the OpenHands adapter isolated behind one crate boundary
- go WebSocket-first for runtime updates
- local supervised mode is the MVP
- hosted mode is a follow-on milestone, not an MVP blocker, so it will be built in the future
- every task should be independently implementable by a coding agent with only repository and task context

## 2. Milestones

| Milestone | Scope | Outcome |
|---|---|---|
| M1 Foundation and contracts | repo bootstrap, workflow/config, domain model | stable repo skeleton and core invariants |
| M2 OpenHands runtime adapter | local supervisor, REST client, WebSocket runtime, session runner | working agent runtime path |
| M3 Symphony orchestration core | workspace lifecycle, Linear adapter, scheduler, GraphQL-backed repo harness | issue-driven autonomous execution with a generic scheduler core over tracker, workspace, and worker backends |
| M4 Operator UX and repo harness | control plane, FrankenTUI, generated context artifacts | usable local operator experience |
| M5 Validation and local packaging | fake server, live tests, doctor, packaging | reliable local MVP |

## 3. Issue hierarchy

```text
OSYM-100 Foundation and Contracts
  ├─ OSYM-101 Bootstrap workspace and crate boundaries
  ├─ OSYM-102 Workflow loader and typed config
  └─ OSYM-103 Domain model and orchestrator state machine

OSYM-200 OpenHands Runtime Adapter
  ├─ OSYM-201 Local agent-server supervisor
  ├─ OSYM-202 REST client and conversation contract
  ├─ OSYM-203 WebSocket event stream, reconciliation, and recovery
  └─ OSYM-204 Issue session runner

OSYM-300 Tracker, Workspaces, and Orchestration
  ├─ OSYM-301 Workspace manager and lifecycle hooks
  ├─ OSYM-302 Linear read adapter
  ├─ OSYM-303 Linear GraphQL agent write path
  ├─ OSYM-304 Orchestrator scheduler, retries, and reconciliation
  └─ OSYM-305 Repository harness and generated context artifacts

OSYM-400 Observability and FrankenTUI
  ├─ OSYM-401 Control-plane API and snapshot store
  └─ OSYM-402 FrankenTUI operator client

OSYM-500 Validation and Local Ops
  ├─ OSYM-501 Fake OpenHands server and contract suite
  ├─ OSYM-502 Live local end-to-end suite
  └─ OSYM-503 CLI packaging, doctor, and operations docs
```

## 4. Recommended execution order

1. OSYM-101
2. OSYM-102 and OSYM-103
3. OSYM-201 and OSYM-202
4. OSYM-203
5. OSYM-301 and OSYM-302
6. OSYM-204
7. OSYM-303 and OSYM-304
8. OSYM-401
9. OSYM-305 and OSYM-402
10. OSYM-501
11. OSYM-502 and OSYM-503

## 5. Dependency graph

```text
OSYM-101
  ├─ OSYM-102
  ├─ OSYM-103
  ├─ OSYM-201
  ├─ OSYM-202
  ├─ OSYM-301
  └─ OSYM-302

OSYM-102 + OSYM-103
  └─ OSYM-204

OSYM-201 + OSYM-202
  └─ OSYM-203

OSYM-203 + OSYM-301 + OSYM-302 + OSYM-102 + OSYM-103
  └─ OSYM-204

OSYM-302
  └─ OSYM-303

OSYM-204 + OSYM-301 + OSYM-302
  └─ OSYM-304

OSYM-304
  └─ OSYM-401

OSYM-303 + OSYM-304
  └─ OSYM-305

OSYM-401
  └─ OSYM-402

OSYM-202 + OSYM-203 + OSYM-204 + OSYM-302 + OSYM-304
  └─ OSYM-501

OSYM-201 + OSYM-204 + OSYM-303 + OSYM-304 + OSYM-305 + OSYM-501
  └─ OSYM-502

OSYM-401 + OSYM-402 + OSYM-502
  └─ OSYM-503
```

## 6. Linear conversion guidance

Each task file in `docs/tasks/` includes front matter intended for issue creation.

Recommended mapping:

- `id` -> issue identifier suffix or custom field
- `title` -> issue title
- `parent` -> parent issue relationship
- `milestone` -> Linear project milestone
- `priority` -> Linear priority
- `estimate` -> estimate field or label
- `depends_on` -> linked blocking issues
- `blocks` -> reverse dependency links
- `project_context` -> issue description reference block
- `repo_paths` -> code ownership hint inside issue body
- `definition_of_ready` -> checklist in issue body

## 7. Definition of Ready

A task is ready when:

- its dependencies are merged or available in the working branch
- the docs referenced in `project_context` exist and are current
- target repository paths are identified
- required test fixtures are available
- required secrets or local services for live tests are documented

## 8. Definition of Done for the local MVP

The local MVP is done when all of the following are true:

- the daemon can read active Linear issues
- issue workspaces are created deterministically and safely
- a local supervised OpenHands server can execute issue conversations with per-issue `working_dir`
- the runtime adapter is WebSocket-first with readiness, reconciliation, and reconnect recovery
- the orchestrator can claim, run, retry, reconcile, and release work according to Symphony semantics
- the local control plane publishes snapshots
- FrankenTUI can observe and render daemon state
- fake-server contract tests pass
- live local end-to-end tests pass on a controlled machine
- `opensymphony doctor` validates a machine well enough to run the MVP

## 9. Parent issue strategy

Create the parent issues first in Linear, attach the relevant milestone, then create the child issues and link them under the parent.

Recommended parent issue purpose:

- collect architectural context
- hold milestone progress
- aggregate child acceptance notes
- reduce duplication in child issue descriptions

## 10. Suggested first milestone review gate

At the end of M2, run a formal adapter review.

Required evidence:

- conversation create payload example
- successful WebSocket attach trace
- readiness event handling proof
- reconcile-after-ready proof
- disconnect and reconnect test output
- one end-to-end run in a temp repo

If M2 is solid, M3 onward is mostly orchestration work rather than protocol risk.

## 11. Follow-on rich client and knowledge milestones

The current Linear roadmap extends the local MVP with rich-client, hosted,
provider, and release-quality work in M6 through M13. The additional OKF memory,
LLM wiki graph, and ACP debugging specifications fit best as additive fractional
milestones instead of broadening the existing hardening milestone:

| Milestone | Placement | Scope |
|---|---|---|
| M10.5 OKF Memory Bundle Foundation | after M10 web/client transport, before M11 hosted visibility depends on portable memory documents | OKF concept schema, writer/lint, catalog reindex, import/export, docs sync and MCP/admin parity |
| M11.5 LLM Wiki Graph View | after M11 hosted identity and visibility foundations, before provider/harness readiness | memory graph DTOs, graph extraction, shared frontend package, Three.js renderer, inspector/accessibility, live privacy gates |
| M12.5 ACP Debugging And IDE Attach | after M12 harness/model seams, before M13 release hardening | debug attachment refactor, ACP stdio server, Zed setup, Tauri Debug in Zed action, default debug UX transition, integration tests |

The planning source for this wave is
`docs/tasks/advanced-knowledge-debug-task-package.yaml`. The existing
`docs/tasks/task-package.yaml` remains the published source for the
rich-client-hosted-mode wave.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-252 contributed: PR #10: Implement foundation workflow and scheduler contracts
- COE-253 contributed: PR #19: COE-253: OpenHands Runtime Adapter (merge `911b0b4`)
- COE-254 contributed: PR #6: COE-254: bootstrap tracker, workspace, and orchestration core
- COE-255 contributed: PR #4: COE-255: add control plane and FrankenTUI slice
- COE-256 contributed: PR #1: COE-257: tighten hosted deployment guidance
- COE-258 contributed: PR #83: Add memory init and mapped docs sync

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-252: Foundation and Contracts
- COE-253: OpenHands Runtime Adapter
- COE-254: Tracker, Workspaces, and Orchestration
- COE-255: Observability and FrankenTUI
- COE-256: Validation and Local Operations
- COE-258: Bootstrap workspace and crate boundaries
- COE-259: Workflow loader and typed config
- COE-260: Domain model and orchestrator state machine
- COE-261: Local agent-server supervisor
- COE-262: REST client and conversation contract
- COE-263: Workspace manager and lifecycle hooks
- COE-264: Linear read adapter and issue normalization
- COE-265: WebSocket event stream, reconciliation, and recovery
- COE-266: Issue session runner
- COE-267: Linear MCP write surface
- COE-268: Orchestrator scheduler, retries, and reconciliation
- COE-269: Control-plane API and snapshot store
- COE-270: Repository harness and generated context artifacts
- COE-271: FrankenTUI operator client
- COE-272: Fake OpenHands server and protocol contract suite
- COE-273: Live local end-to-end suite
- COE-274: CLI packaging, doctor, and local operations docs
- COE-275: Remote agent-server mode and auth hardening
- COE-277: Implement hierarchy-aware task selection
- COE-280: Support workflow-owned OpenHands auth, provider, and launcher overrides at runtime
- COE-281: Support path-bearing OpenHands base URLs and MCP config at runtime
- COE-282: Support workflow-owned OpenHands conversation reuse policy at runtime
- COE-284: Add orchestrator run command to CLI and make it installable
- COE-287: Add opensymphony debug command for conversational session debugging
- COE-294: Detect LLM config changes and rehydrate conversations with updated env vars
- COE-382: Add supply-chain and security audits to CI
- COE-383: Decompose oversized session and TUI modules into focused submodules
- COE-384: Expand error-path tests for Linear client and workspace hooks
- COE-385: Resolve runtime tracking TODO in OpenHands session runner
- COE-386: Wire cargo-llvm-cov coverage reporting and regression floor into CI
- COE-387: Audit tracing spans and diagnostics for secret leakage
- COE-389: Current Gateway Inventory And Vocabulary
- COE-390: Gateway Schemas And Stream Feasibility
- COE-391: Gateway Module, Capabilities, And Dashboard Snapshot
- COE-392: Task Graph, Run Detail, File, And Diff Read APIs
- COE-393: Event Journal And Stream Broker
- COE-394: Frontend Workspace And Shared Schemas
- COE-395: Planning Artifact Schema And Session Service
- COE-396: Action Receipts And Initial Run Actions
- COE-397: Gateway API Client, Transport Adapters, And Reducers
- COE-398: Tauri Shell And Security Capabilities
- COE-399: Linear Read Coverage And Task Graph Cache
- COE-400: OpenHands Event Normalization And Runtime Mirror
- COE-401: Web App Entry And Deployment Modes
- COE-402: App Shell, Dashboard, Task Graph, And Run Views
- COE-403: Terminal And Log Renderer Prototype
- COE-404: Desktop Connection Profiles And Daemon Management
- COE-405: Linear Milestone, Issue, And Sub-Issue Mutations
- COE-406: Repository, Linear, And Research Analysis
- COE-409: Desktop Settings, Keychain, And Native Actions
- COE-410: Desktop Local Stream Optimization
- COE-411: Task Graph Editor And Runtime Overlay UI
- COE-412: Runtime Timeline And Terminal/Log Association
- COE-413: Implementation Plan Generator Stage
- COE-414: Diff, Validation, Approval, And Run Action Views
- COE-415: Milestone, Issue, And Sub-Issue Compiler
- COE-416: Dependency Graph And Plan Checks
- COE-417: Planning Workspace UI
- COE-434: Long-running harness liveness and scheduler/runtime ownership contract
- COE-435: Long-running run observability fixtures and client-facing diagnostics
- COE-448: Multi-repo memory server and deterministic context
- COE-449: Desktop alpha recovery: replace stubs with functional app

## Source refs

- COE-252
- COE-253
- COE-254
- COE-255
- COE-256
- COE-258
- COE-259
- COE-260
- COE-261
- COE-262
- COE-263
- COE-264
- COE-265
- COE-266
- COE-267
- COE-268
- COE-269
- COE-270
- COE-271
- COE-272
- COE-273
- COE-274
- COE-275
- COE-277
- COE-280
- COE-281
- COE-282
- COE-284
- COE-287
- COE-294
- COE-382
- COE-383
- COE-384
- COE-385
- COE-386
- COE-387
- COE-389
- COE-390
- COE-391
- COE-392
- COE-393
- COE-394
- COE-395
- COE-396
- COE-397
- COE-398
- COE-399
- COE-400
- COE-401
- COE-402
- COE-403
- COE-404
- COE-405
- COE-406
- COE-409
- COE-410
- COE-411
- COE-412
- COE-413
- COE-414
- COE-415
- COE-416
- COE-417
- COE-434
- COE-435
- COE-448
- COE-449

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
