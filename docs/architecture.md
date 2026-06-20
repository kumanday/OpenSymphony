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
   - feature-gated Codex app-server prototype adapter
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
- `opensymphony_codex`
  - feature-gated Codex app-server launch, JSON-RPC, credential reuse, and
    benchmark prototype helpers
- `opensymphony_orchestrator`
  - scheduler loop and reconciliation
- `opensymphony_control`
  - control-plane snapshot store and compatibility API
- `opensymphony_gateway`
  - operator gateway API, dashboard snapshots, Linear-backed task graph reads,
    run detail/file/diff endpoints, event journal, and web assets
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

## 5.1 Gateway and rich clients

The web and desktop clients consume the gateway contract rather than reaching
into orchestrator internals. Dashboard and run state come from the
control-plane snapshot, while the task graph read endpoint asks the
orchestrator-side Linear adapter for tracker hierarchy and dependency
relationships, then overlays live runtime details from the latest snapshot.
The gateway emits `root_ids` from the returned Linear parent/child graph so
clients can render the same forest without inventing hierarchy locally.
If the optional task graph reader cannot be built, `opensymphony run` still
starts the gateway and the task graph endpoint returns `503`; this does not
weaken the scheduler's separate Linear tracker requirement.

Native desktop builds may call the same operations through Tauri IPC instead
of loopback HTTP, but the data contract is identical. Tauri command arguments
use the Rust command parameter names exactly, including snake_case keys such as
`run_id`, `project_id`, `page_token`, `page_size`, and `file_path`. If a native
desktop read command fails, the desktop adapter may retry through the loopback
HTTP transport for the same gateway operation.
Run-event `page_token` values are gateway-generated sequence tokens encoded as
strings; malformed tokens are rejected with `400 Bad Request` instead of being
silently treated as the first page.

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

- COE-426 contributed: PR #131: Add Codex app-server prototype benchmark (merge `90ce68d`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-426: Codex App-Server Prototype And Benchmarks

## Source refs

- COE-426

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
