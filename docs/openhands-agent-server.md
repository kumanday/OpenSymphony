# OpenHands Agent-Server Integration

## 1. Chosen integration surface

OpenSymphony integrates with the OpenHands SDK agent-server surface from Rust.

In scope:

- conversation create/get/send/run
- event search
- runtime event streaming over WebSocket
- workspace-aware conversation launch
- local supervised mode and external-server mode

Out of scope:

- `openhands serve`
- the web app Socket.IO protocol
- browser-oriented client integrations

## 2. Why OpenHands fits

OpenHands provides the execution-layer primitives Symphony needs:

- per-conversation workspace path
- provider-agnostic model configuration
- persistent conversations
- structured events
- background runs
- recoverable state over HTTP and WebSocket

OpenSymphony keeps all scheduling rules in Rust and uses OpenHands only as the
runtime substrate.

## 3. Runtime modes

### Local supervised mode

The daemon launches one local OpenHands agent-server subprocess.

Use for:

- single-developer setup
- local experimentation
- CI smoke environments

Repository ownership:

- `tools/openhands-server/` owns the local package pin
- published `opensymphony` bundles those pinned files and materializes them into
  `~/.opensymphony/openhands-server` via `opensymphony install openhands`
- `run-local.sh` launches the pinned server
- the supervisor probes readiness before treating the server as usable

### External server mode

The daemon connects to an already-running agent-server at
`openhands.transport.base_url`.

Use for:

- pinned external integration tests
- future hosted deployments
- organization-managed runtime infrastructure

## 4. Startup contract

In supervised mode:

1. resolve launch command and environment
2. start the subprocess on loopback
3. probe readiness
4. create conversations with workflow-owned settings
5. reuse the server across issue runs

Rules:

- prefer real readiness probes over fixed sleeps
- do not silently adopt a foreign process as an owned child
- normalize local path-prefixed loopback targets before launch when needed

## 5. Workspace model

OpenSymphony uses one server with many issue-specific workspaces.

Each conversation request sets `workspace.working_dir` to the deterministic
issue workspace path. This preserves issue isolation without requiring a
separate server per issue.

## 6. Conversation model

Each issue gets a stable conversation manifest under:

```text
<issue_workspace>/.opensymphony/conversation.json
```

That manifest records enough data for:

- reattachment
- restart recovery
- debug reuse
- rehydration

The persisted OpenHands state directory is derived from the workflow-owned
conversation persistence settings inside the issue workspace.

In managed local mode, the OpenHands server's global conversation registry is
also scoped per target repo. OpenSymphony sets `OH_CONVERSATIONS_PATH` to:

```text
<tool_dir>/workspace/conversations/repos/<repo-key>/active
```

Before managed server startup, known terminal issue conversations from existing
workspace manifests move into the sibling `archived` store, and current Linear
candidate issues move into `active` from `archived` or the legacy flat store.
This startup migration is an isolated compatibility shim for pre repo-scoped
stores and can be removed after existing installs have aged out. It prevents the
managed server from eagerly resuming every historical conversation across all
repos during normal orchestration. `opensymphony debug <issue-id>` locates the
requested conversation in active or archived storage and starts the managed
server against that store before attaching.

The default managed-local startup window is 180 seconds. Debug-launched managed
servers suppress raw agent-server stderr during readiness so restored
conversation state does not flood the operator terminal before the debug
transcript renderer takes over.

## 7. Runtime contract

The internal `opensymphony_openhands` module owns:

- typed request/response models
- authenticated REST requests when needed
- WebSocket attach/reconcile/reconnect behavior
- ready-state detection
- issue session launch and reuse

For reused conversations that are already `queued` or `running`, the runtime now
surfaces launch metadata immediately after a successful attach so the
orchestrator can keep tracking the live worker while it waits for the previous
turn to settle. The same early launch reporting now also applies when a reused
conversation only reveals its active prior turn later through a `/run`
`409 Conflict`, after the attach-time mirror looked idle.

The orchestration layer should not need to know OpenHands wire details.

## 8. Tooling note

OpenSymphony no longer forwards workflow-owned Linear bridge configuration into
conversation creation.

Agent-side Linear operations now live entirely in the target repo’s checked-in
GraphQL helper assets and use `LINEAR_API_KEY` directly.

## 9. Validation

Important checks:

- `cargo test --test live_pinned_server`
- `cargo test --test issue_session_runner`
- `cargo test --test doctor`

When validating a local setup, confirm that:

- the configured OpenHands target is reachable
- a temp conversation can be created
- the WebSocket stream reaches readiness
- restart recovery reuses the stored conversation manifest correctly

## 10. Migration note

OpenSymphony 1.0.0 removed workflow-owned `openhands.mcp`. Older repos should
remove that block and rely on the repo-local Linear GraphQL helper assets
copied by `opensymphony init`.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-408 contributed: PR #129: Add harness adapter capability discovery (merge `96345e3`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-408: Harness Adapter And Capability Model

## Source refs

- COE-408

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
