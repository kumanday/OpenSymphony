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
| M10.3 Codex And Subscription Readiness | after M10 web client/external gateway work, before M10.5 OKF memory and M11 hosted alpha | local Codex app-server support and ChatGPT subscription credential foundations before full hosted mode |
| M10.5 OKF Memory Bundle Foundation | after M10 web/client transport, before M11 hosted visibility depends on portable memory documents | OKF concept schema, writer/lint, catalog reindex, import/export, docs sync and MCP/admin parity |
| M11.5 LLM Wiki Graph View | after M11 hosted identity and visibility foundations, before provider/harness readiness | memory graph DTOs, graph extraction, shared frontend package, Three.js renderer, inspector/accessibility, live privacy gates |
| M12.5 ACP Debugging And IDE Attach | after M12 harness/model seams, before M13 release hardening | debug attachment refactor, ACP stdio server, Zed setup, Tauri Debug in Zed action, default debug UX transition, integration tests |

M10.3 pulls the shared harness adapter, model/credential settings, OpenHands
subscription credential adapter, model configuration UI metadata, Codex
app-server prototype, ChatGPT OAuth readiness, production Codex harness
enablement, and cross-harness approval/routing work forward before the broader
hosted-provider backlog. Hosted subscription credential brokering remains in
M11 so local Codex subscription support is not blocked by hosted secret storage.

The planning source for this wave is
`docs/tasks/advanced-knowledge-debug-task-package.yaml`. The existing
`docs/tasks/task-package.yaml` remains the published source for the
rich-client-hosted-mode wave.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-476 contributed: PR #136: Enable local Codex app-server harness (merge `303ab81`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-476: Codex Production Harness Enablement

## Source refs

- COE-476

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
