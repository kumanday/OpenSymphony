# Architecture

## 1. Objective

Implement the Symphony orchestration model in Rust while using OpenHands as the
execution substrate and FrankenTUI as an optional operator client.

The system must preserve these boundaries:

- the orchestrator is the source of truth for scheduling state
- the tracker is polled and reconciled by the orchestrator
- each issue executes in its own workspace
- `WORKFLOW.md` remains the repo-owned policy and prompt contract
- UI is optional and must not affect correctness

## 2. Layered design

OpenSymphony is split into five layers:

1. Policy layer
   - `WORKFLOW.md`
   - target-repo `AGENTS.md`
   - target-repo `.agents/skills/`
2. Configuration layer
   - typed workflow/config loader
   - env and path resolution
   - OpenHands extension config
3. Coordination layer
   - orchestrator actor
   - retry queue
   - reconciliation
   - runtime snapshot store
4. Execution layer
   - workspace manager
   - OpenHands REST client
   - OpenHands WebSocket runtime stream
   - issue session runner
5. Observability layer
   - structured logs
   - control-plane API
   - FrankenTUI

Packaging distinction:

- modularity is preserved through explicit internal subsystem boundaries
- packaging is intentionally flat: crates.io publishes only `opensymphony`
- the `crates/opensymphony-*` directories are internal module trees compiled
  into that one package

## 3. Main decisions

### 3.1 Rust owns orchestration

Rust owns:

- poll cadence
- issue eligibility
- bounded concurrency
- retry scheduling
- stall detection
- startup cleanup
- restart recovery
- operator snapshots

OpenHands conversation state is informative, not authoritative.

### 3.2 OpenHands is the execution adapter

OpenHands provides:

- per-conversation workspace configuration
- persistent conversations
- background run triggering
- searchable event history
- real-time updates over WebSocket
- provider/model flexibility

OpenSymphony does not reimplement an agent loop.

### 3.3 WebSocket-first, not WebSocket-only

REST is still required for:

- conversation creation
- sending messages
- triggering runs
- initial sync
- reconnect reconciliation
- restart recovery

### 3.4 One local server, many workspaces

The local supervised topology runs one OpenHands server for the daemon while
passing a distinct `working_dir` per issue.

### 3.5 One conversation per issue by default

OpenSymphony persists a stable `conversation_id` per issue inside the issue
workspace and reuses it across retries and daemon restarts unless the workflow
reuse policy says otherwise.

### 3.6 GraphQL-only Linear writes

OpenSymphony 1.0.0 removed the old bridge layer for agent-side Linear writes.

The supported model is now:

- orchestrator reads Linear through the internal `opensymphony_linear` module
- initialized target repos read and write Linear through the checked-in
  GraphQL helper assets under `.agents/skills/linear/`

This keeps one canonical Linear API surface for the agent path.

## 4. Component model

### Internal subsystem modules

- `opensymphony_domain`
  - domain models and scheduler transitions
- `opensymphony_workflow`
  - workflow loading, config resolution, prompt rendering
- `opensymphony_workspace`
  - workspace management and manifests
- `opensymphony_linear`
  - Linear GraphQL read adapter and guarded archive mutation
- `opensymphony_memory`
  - issue capsules, DuckDB memory index, docs sync, and archive eligibility
- `opensymphony_openhands`
  - OpenHands transport and session runner
- `opensymphony_orchestrator`
  - scheduler loop and reconciliation
- `opensymphony_control`
  - control-plane snapshot store and compatibility API
- `opensymphony_gateway`
  - operator gateway API, dashboard snapshots, event journal, and web assets
- `opensymphony_cli`
  - user-facing entrypoints
- `opensymphony_tui`
  - terminal operator UI
- `opensymphony_testkit`
  - fakes and contract fixtures

### Target-repo Linear assets

Initialized repositories receive a checked-in Linear skill tree:

- `SKILL.md`
- `scripts/linear_graphql.py`
- `queries/*.graphql`
- `references/*.md`

Those assets are part of the supported public interface of `opensymphony init`.
They include canonical query files for issue create/update flows, comments,
relations, attachments, project content/status updates, and introspection.

## 5. Process model

Local MVP process graph:

```text
opensymphony run
  ├─ orchestrator
  ├─ workspace manager
  ├─ linear adapter
  ├─ openhands REST client
  ├─ openhands WebSocket client
  ├─ gateway API
  ├─ control-plane compatibility API
  └─ local server supervisor
       └─ python -m openhands.agent_server
```

Other processes:

- `opensymphony debug <issue-id>`
- `opensymphony tui`
- target-repo hooks started by the workspace manager
- OpenHands-managed tool execution inside the agent runtime

There is no separate agent-side Linear bridge process in 1.0.0.

## 6. Failure boundaries

- scheduler correctness must not depend on tracker comments or transitions
- GraphQL write failures in the target repo do not corrupt orchestrator state
- a missing `LINEAR_API_KEY` blocks Linear operations but should fail clearly
- UI failures must not affect daemon execution

## 7. Migration boundary

OpenSymphony 1.0.0 is the compatibility boundary for the GraphQL-only Linear
rewrite and the provider-agnostic AI review configuration changes.

Notable removals:

- workflow-owned `openhands.mcp`
- the old bridge CLI command
- provider-specific AI review secret naming

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-389 contributed: PR #85: docs: gateway inventory, domain vocabulary, and DTO boundary checklist (COE-389) (merge `3ed56af`)
- COE-390 contributed: PR #85: docs: gateway inventory, domain vocabulary, and DTO boundary checklist (COE-389) (merge `3ed56af`)
- COE-391 contributed: PR #85: docs: gateway inventory, domain vocabulary, and DTO boundary checklist (COE-389) (merge `3ed56af`)
- COE-392 contributed: PR #85: docs: gateway inventory, domain vocabulary, and DTO boundary checklist (COE-389) (merge `3ed56af`)
- COE-393 contributed: PR #91: feat: Event Journal and Stream Broker (COE-393) (merge `1183bc6`)
- COE-394 contributed: PR #89: COE-394: Frontend workspace and shared schemas (merge `68d86ff`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

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
- COE-449: Desktop alpha recovery: replace stubs with functional app

## Source refs

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
- COE-449

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
