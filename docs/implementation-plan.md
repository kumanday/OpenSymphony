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
LLM wiki graph, ACP debugging, and Tree-sitter code-intelligence specifications fit best as additive fractional
milestones instead of broadening the existing hardening milestone:

| Milestone | Placement | Scope |
|---|---|---|
| M10.3 Codex And Subscription Readiness | after M10 web client/external gateway work, before M10.5 OKF memory and M11 hosted alpha | local Codex app-server support and ChatGPT subscription credential foundations before full hosted mode |
| M10.4 Desktop Live Operations And Model Polish | after M10.3 Codex readiness, before M10.5 OKF memory | desktop live refresh, truthful run-detail metrics, and Codex subscription model-configuration cleanup from first operator use |
| M10.5 OKF Memory Bundle Foundation | after M10 web/client transport, before M11 hosted visibility depends on portable memory documents | OKF concept schema, writer/lint, catalog reindex, import/export, docs sync and MCP/admin parity |
| M10.6 Desktop Run Detail Operations And Interrupts | after M10.5 OKF memory, before M11 hosted alpha | harness interrupt contract, OpenHands/Codex cancel wiring, Human Review to Merging supersede, Run Detail action cleanup, TUI parity, lazy desktop launcher |
| M10.7 Project Grouping And Dependency Signals | after M10.6 desktop operations, before M11 hosted alpha | control-plane project/dependency metadata, TUI project headers and dependency gutter, desktop grouping and collapse |
| M11.5 LLM Wiki Graph View | after M11 hosted identity and visibility foundations, before provider/harness readiness | memory graph DTOs, graph extraction, shared frontend package, Three.js renderer, inspector/accessibility, live privacy gates |
| M12.5 ACP Debugging And IDE Attach | after M12 harness/model seams, before M13 release hardening | debug attachment refactor, ACP stdio server, Zed setup, Tauri Debug in Zed action, default debug UX transition, integration tests |
| M12.6 Tree-sitter Code Intelligence | after M12.5 ACP debugging, before M13 release hardening | trusted Tree-sitter provider, memory-context AST integration, query packs, persistence, read-only MCP/CLI AST tools, performance and docs |

M10.3 pulls the shared harness adapter, model/credential settings, OpenHands
subscription credential adapter, model configuration UI metadata, Codex
app-server prototype, ChatGPT OAuth readiness, production Codex harness
enablement, and cross-harness approval/routing work forward before the broader
hosted-provider backlog. Hosted subscription credential brokering remains in
M11 so local Codex subscription support is not blocked by hosted secret storage.

M10.4 captures the immediate operator feedback from live Codex-backed desktop
and TUI use: live snapshot/run-detail refresh without app restart, truthful Run
Detail metrics, model configuration defaults/auth-status cleanup for Codex
subscription profiles, Codex token usage accounting, and more useful Codex
event summaries.

M10.6 follows the desktop operations spec with the smallest set of issue-ready
tasks for real cancellation, same-conversation Human Review to Merging
supersede handling, Run Detail button cleanup, TUI parity, and the lazy desktop
launcher command.

M10.7 follows the TUI dependency gutter spec with project/dependency snapshot
fields, compact FrankenTUI project headers and dependency signals, desktop
project grouping with collapse, and a thin cross-client verification pass.

M12.6 follows the Tree-sitter AST specification with the smallest useful
agent-facing code-intelligence wave: a trusted provider, memory-context
integration, query packs, optional persistence, read-only AST tools, and
resource/security hardening.

The current default planning source is `docs/tasks/task-package.yaml` for the
M12.6 tree-sitter-code-intelligence wave. The ACP debugging wave remains
in `docs/tasks/advanced-knowledge-debug-task-package.yaml`, and
`docs/tasks/linear-publish.yaml` records the latest published planning wave.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-401 contributed: PR #92: COE-401: Web app entry and deployment modes (merge `73b9067`)
- COE-407 contributed: PR #125: feat(api-client): browser transport streaming, replay, and remote protocols (COE-407) (merge `4d70347`)
- COE-408 contributed: PR #129: Add harness adapter capability discovery (merge `96345e3`)
- COE-419 contributed: PR #126: Load desktop task graph dependencies from Linear (merge `64242a6`)
- COE-423 contributed: PR #130: feat(gateway): add model credential settings seam (merge `07274f4`)
- COE-425 contributed: PR #132: feat(openhands): add subscription credential adapter (merge `93cea67`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-401: Web App Entry And Deployment Modes
- COE-407: Browser Transport And Remote Stream Protocols
- COE-408: Harness Adapter And Capability Model
- COE-419: Hosted Auth Placeholders And Web Parity
- COE-423: Model And Credential Settings
- COE-425: OpenHands Subscription Credential Adapter
- COE-426: Codex App-Server Prototype And Benchmarks
- COE-428: Model Configuration UI And Routing Metadata
- COE-429: Codex Approvals And Cross-Harness Routing
- COE-454: OKF Bundle Schema And Legacy Capsule Mapping
- COE-456: OKF Writer, Lint, And Migration Fixtures
- COE-458: Catalog Reindex And Query Compatibility From OKF
- COE-460: OKF Export, Import, And Visibility Boundaries
- COE-463: Docs Sync And MCP Admin Parity For OKF
- COE-473: Desktop task graph dependency and run detail parity
- COE-475: ChatGPT OAuth For Codex Harness
- COE-476: Codex Production Harness Enablement
- COE-478: Harden model profile storage and validation follow-ups
- COE-479: Codex Debug Session Resume

## Source refs

- COE-401
- COE-407
- COE-408
- COE-419
- COE-423
- COE-425
- COE-426
- COE-428
- COE-429
- COE-454
- COE-456
- COE-458
- COE-460
- COE-463
- COE-473
- COE-475
- COE-476
- COE-478
- COE-479

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
