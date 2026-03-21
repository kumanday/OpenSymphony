# AGENTS.md

## Mission

Build OpenSymphony as a Rust implementation of the Symphony service specification using OpenHands agent-server for execution and FrankenTUI for the optional terminal UI.

This repository is an orchestrator. It is not a chat app, not a general workflow engine, and not a thin wrapper around OpenHands.

## Authority order

When sources disagree, use this order:

1. upstream `openai/symphony` `SPEC.md`
2. pinned OpenHands SDK agent-server docs and the wire-contract notes in `docs/websocket-runtime.md`
3. this repository's `docs/`
4. the task file currently being implemented
5. local code comments and tests

Do not silently invent behavior when the upstream spec or chosen integration contract is explicit.

## Hard invariants

### Orchestration

- The Rust orchestrator is the sole authority over scheduling state.
- Workers report events and outcomes to the orchestrator.
- No background task may mutate scheduling state except through orchestrator-owned commands or messages.
- Tracker polling remains required even though agent runtime updates use WebSockets.

### Workspace safety

- Every issue maps to exactly one sanitized workspace key.
- Workspace paths must remain inside the configured workspace root.
- The agent runtime must execute with `cwd == issue_workspace_path`.
- Never run agent code in the orchestrator repository root, temp root, or an unsanitized path.

### OpenHands integration

- Target the SDK agent-server HTTP and WebSocket contract.
- Do not implement against `openhands serve`.
- Do not implement against the web-app Socket.IO protocol.
- Operations are REST. Runtime streaming is WebSocket.
- The WebSocket readiness barrier is the first `ConversationStateUpdateEvent`.
- Always reconcile events after WebSocket readiness and after reconnect.
- One OpenHands conversation is reused per issue by default.
- A fresh conversation gets the full workflow prompt. A resumed conversation gets continuation guidance only.
- Local MVP uses one local agent-server subprocess shared across issues, not one server per issue.
- Local MVP does not require Docker per workspace.

### Tracker contract

- The orchestrator reads Linear directly.
- Tracker writes are done by agent-side tools through MCP unless a future operator API explicitly documents otherwise.
- Scheduler correctness must not depend on agent-side tracker writes succeeding.

### UI separation

- FrankenTUI is optional.
- The daemon must remain correct without any UI attached.
- The UI consumes the control-plane snapshot and event stream only.
- UI code must not reach into orchestrator internals directly.

## Design rules

### Keep boundaries explicit

Preferred crate and trait boundaries:

- `opensymphony-domain`
- `opensymphony-workflow`
- `opensymphony-workspace`
- `opensymphony-linear`
- `opensymphony-linear-mcp`
- `opensymphony-openhands`
- `opensymphony-orchestrator`
- `opensymphony-control`
- `opensymphony-cli`
- `opensymphony-tui`
- `opensymphony-testkit`

Add new crates only when there is a clear ownership boundary.

### Prefer actor ownership over shared locks

The orchestrator should own mutable runtime state in one async task.

Use channels and message passing for worker reports, retries, and control-plane publication.

Avoid spreading `Arc<Mutex<...>>` through the daemon.

### Keep the WebSocket client resilient

The runtime client must:

- connect after conversation creation
- wait for readiness
- reconcile the REST event backlog
- deduplicate by event ID
- preserve timestamp order
- reconnect with bounded exponential backoff
- refresh cached state after reconnect

### Preserve forward compatibility

OpenHands event schemas can evolve. Implement:

- typed decoding for known high-value events
- raw JSON retention for unknown events
- compatibility tests against the pinned version
- version notes in `docs/sources.md`

### Separate Symphony hooks from OpenHands hooks

Symphony workspace hooks:

- `after_create`
- `before_run`
- `after_run`
- `before_remove`

These are owned by OpenSymphony.

OpenHands hook configuration such as `pre_tool_use` is a separate, optional agent runtime feature. Do not conflate them.

## Local safety posture

The local MVP is a trusted-environment mode.

- Expect host filesystem access.
- Expect host process execution.
- Do not overstate isolation.
- Harden later for hosted mode with remote or container-backed workspaces.
- Document risky defaults clearly in `README.md` and `docs/testing-and-operations.md`.

## Coding standards

- Rust stable toolchain
- `cargo fmt` clean
- `clippy` clean under repo lints
- explicit error enums with context
- structured logs, not ad hoc print-only debugging
- `tokio` cancellation handled deliberately
- serde models for all external payloads
- integration code isolated inside `opensymphony-openhands`
- no direct OpenHands protocol types leaking into orchestrator core types

## Required tests by subsystem

### Workflow and config

- front matter parsing
- strict template rendering failure modes
- env indirection
- extension namespace validation
- path normalization

### Workspace

- identifier sanitization
- containment checks
- hook timeout handling
- create and reuse semantics
- terminal cleanup behavior

### OpenHands runtime

- conversation create payload
- event send and run trigger
- WebSocket readiness
- event reconciliation
- out-of-order event ordering
- reconnect and replay
- terminal `execution_status` detection
- conversation reuse and reset paths

### Orchestrator

- candidate sorting
- active vs terminal reconciliation
- bounded concurrency
- normal continuation retry
- failure backoff
- stall detection
- restart recovery

### Control plane and TUI

- snapshot derivation
- control-plane API serialization
- no daemon mutation from UI
- pane layout state
- log and event rendering

## Change-management rules

When changing behavior in any of these files, update the corresponding docs in the same change:

- `docs/architecture.md`
- `docs/openhands-agent-server.md`
- `docs/websocket-runtime.md`
- `docs/workspace-and-lifecycle.md`
- `docs/linear-and-tools.md`
- `docs/testing-and-operations.md`

When changing milestones or task sequencing, update `docs/implementation-plan.md`.

When changing the pinned OpenHands assumptions, update `docs/sources.md`.

## File map

- `README.md`: project summary and implementation path
- `docs/architecture.md`: runtime architecture
- `docs/symphony-spec-alignment.md`: upstream spec mapping
- `docs/openhands-agent-server.md`: agent-server integration choices
- `docs/websocket-runtime.md`: wire contract and recovery behavior
- `docs/workspace-and-lifecycle.md`: workspace ownership and hooks
- `docs/linear-and-tools.md`: Linear integration and MCP tools
- `docs/ui-frankentui.md`: operator UI design
- `docs/repository-layout.md`: crate ownership
- `docs/deployment-modes.md`: local MVP and hosted follow-on
- `docs/testing-and-operations.md`: tests, doctor, packaging, local ops
- `docs/tasks/`: issue-ready implementation work items
