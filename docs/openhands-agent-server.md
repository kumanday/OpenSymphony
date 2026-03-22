# OpenHands Agent-Server Integration

## 1. Chosen integration surface

OpenSymphony integrates directly with the OpenHands SDK agent-server surface from Rust.

### In scope

- SDK agent-server docs under `docs.openhands.dev/sdk/guides/agent-server`
- SDK architecture docs under `docs.openhands.dev/sdk/arch`
- SDK API reference pages for conversations, events, and workspace
- SDK source for `RemoteConversation` only when wire-level details are missing from docs

### Out of scope for the MVP runtime adapter

- `openhands serve`
- OpenHands web-app REST API
- OpenHands web-app Socket.IO WebSocket protocol
- ACP
- browser-oriented client integrations

This distinction matters. The SDK agent-server contract is the clean, language-neutral path for direct Rust orchestration.

## 2. Why OpenHands fits Symphony

OpenHands provides the missing execution-layer primitives that Symphony needs:

- per-conversation workspace path
- provider-agnostic model configuration
- persistent conversations
- tool and MCP support
- structured events
- remote background run trigger
- recoverable state over HTTP

OpenSymphony keeps all Symphony-specific scheduling rules in Rust and uses OpenHands only as the execution substrate.

## 3. Local MVP topology

```text
opensymphony daemon
  ├─ orchestrator
  ├─ workspace manager
  ├─ linear adapter
  ├─ openhands REST client
  ├─ openhands WS client
  ├─ control-plane API
  └─ local server supervisor
       └─ uv run --project tools/openhands-server agent-server --host 127.0.0.1 --port 8000
```

Key properties:

- one local OpenHands server process per daemon
- many issue-specific workspaces via conversation `working_dir`
- no per-issue Docker in the MVP
- host-local execution assumptions documented explicitly

## 4. Server lifecycle model

## 4.1 Modes

OpenSymphony supports two runtime modes through the same trait boundary.

### Local supervised mode

The daemon launches and manages one local OpenHands agent-server subprocess.

Use for:

- single developer setup
- local experimentation
- CI smoke environments with trusted repos

### External server mode

The daemon connects to a pre-existing agent-server at `openhands.transport.base_url`.

`openhands.transport.base_url` may point either at the server root or at the REST-scoped `/api`
prefix. OpenSymphony must normalize root-only probes and WebSocket endpoints back to the server
root in both cases.
All REST calls from this transport must also use bounded connect and request deadlines so a
non-responsive server fails the worker instead of wedging it indefinitely.

Use for:

- integration tests against a pinned external server
- future hosted deployment mode
- organization-managed runtime infrastructure

Local MVP work starts with supervised mode, but the trait boundary must support both.

## 4.2 Startup contract

In supervised mode:

1. Resolve the configured command and environment.
2. Refuse supervised startup if readiness already succeeds before launch, because that indicates an
   external server is already bound to the configured base URL.
3. Start the subprocess bound to `127.0.0.1`.
3. Wait for readiness by probing a known HTTP endpoint.
4. Record server metadata in memory and logs.
5. Reuse the server for all issue runs.

Readiness probing rule:

- prefer the documented `GET /ready` endpoint on the pinned server
- fall back to `GET /health` and then `GET /openapi.json` only if readiness is unavailable
- never rely on sleep-only startup delays
- run readiness probes through the shared HTTP client auth and deadline path rather than issuing
  bespoke unauthenticated requests
- do not claim supervisor ownership of a server that was already ready before the child launch

## 4.3 Shutdown contract

On daemon shutdown:

- stop creating new issue workers
- cancel active workers cleanly
- close live WebSocket clients
- stop the supervised agent-server subprocess if this daemon launched it

If the daemon is using an external server, never attempt to terminate that server.

## 5. Workspace model

The local server guide shows that a remote `Workspace` can be created with both `host` and `working_dir`. This is the central enabler for the MVP:

- one server
- many issue workspaces
- no per-workspace container requirement

OpenSymphony therefore sets `workspace.working_dir` to the deterministic issue workspace path on every conversation creation request.

## 6. Conversation model

## 6.1 Stable conversation identity

Each issue gets a stable `conversation_id` persisted under:

```text
<issue_workspace>/.opensymphony/conversation.json
```

Suggested persisted fields:

- `issue_id`
- `issue_identifier`
- `conversation_id`
- `created_at`
- `last_attached_at`
- `server_base_url`
- `persistence_dir`
- `conversation_contract_version`
- `status`

## 6.2 Persistence directory

Each issue conversation should persist under a stable path inside the issue workspace derived from
`conversation.persistence_dir_relative`, for example:

```text
<issue_workspace>/.opensymphony/openhands/
```

This keeps execution state co-located with the workspace and simplifies recovery while still
allowing operators to move the OpenHands runtime cache within the issue workspace.

## 6.3 Reuse policy

Default policy:

- reuse the conversation for the same issue across worker lifetimes
- if `GET /api/conversations/{id}` briefly returns `404`, retry through
  `POST /api/conversations` with the same `conversation_id` and keep using continuation guidance
  when the persisted thread is rehydrated with existing history
- reset only when:
  - conversation metadata is missing or invalid
  - the server reports the conversation cannot be attached
  - the authoritative conversation `workspace.working_dir` or `persistence_dir` no longer matches
    the current issue workspace
  - an incompatible protocol version is detected
  - an explicit reset policy is configured

## 7. REST endpoints used by OpenSymphony

Use the smallest necessary subset of the agent-server API.

### Required

- `POST /api/conversations`
  - create a conversation
- `GET /api/conversations/{conversation_id}`
  - fetch authoritative conversation state
- `POST /api/conversations/{conversation_id}/events`
  - send a user message event
- `POST /api/conversations/{conversation_id}/run`
  - start background execution
- `GET /api/conversations/{conversation_id}/events/search`
  - sync and reconcile events

### Optional, diagnostic, or future

- model and provider discovery endpoints
- secrets update endpoints
- hook-related endpoints
- bash execute endpoints for diagnostics only

## 8. Conversation creation payload

OpenSymphony should define and validate a minimal typed Rust model for the subset it actually sends.

Required fields:

- `conversation_id`
- `workspace`
  - `working_dir`
  - optional `kind`
- `persistence_dir`
- `max_iterations`
- `stuck_detection`
- `confirmation_policy`
- `agent`
  - minimal stable OpenHands agent payload subset
- optional `hook_config`
- optional `secrets`
- optional `plugins`
- optional `mcp_config`
- `tool_module_qualnames` for every non-builtin tool sent in `agent.tools`

Pinned `v1.14.0` note:

- the server dynamically imports `tool_module_qualnames` on conversation creation and resume
- the server registry names are lower-case identifiers such as `terminal`, `file_editor`, `apply_patch`, `task_tracker`, and `browser_tool_set`
- OpenSymphony may accept class-style aliases such as `TerminalTool` in its own config, but must normalize outbound `agent.tools[*].name` to the server registry names above
- omitting `tool_module_qualnames` causes non-builtin tools to fail with `ToolDefinition '<name>' is not registered`
- builtins such as `FinishTool` and `ThinkTool` remain server-local and do not need module-qualname forwarding

Implementation rule:

- keep the orchestrator core independent of the raw OpenHands JSON model
- all payload shaping belongs in `opensymphony-openhands`

## 9. Authentication

## 9.1 Local MVP

Default local mode:

- bind to loopback only
- no mandatory session API key

## 9.2 Future-proofing

The Rust client should still support:

- no auth
- session API key for HTTP through `X-Session-API-Key`
- session API key for WebSocket query-param fallback
- optional header-based WebSocket auth for versions that support it

Do not assume one auth method forever. Make it configurable and covered by integration tests against the pinned version.

Pinned `v1.14.0` note:

- REST auth uses `X-Session-API-Key`
- WebSocket auth accepts the same header or `session_api_key` as a query parameter

## 10. OpenHands hooks vs Symphony hooks

These are different systems.

### Symphony hooks

Owned by OpenSymphony and run around issue workspace lifecycle:

- `after_create`
- `before_run`
- `after_run`
- `before_remove`

### OpenHands hooks

Owned by OpenHands agent-server and attached through `hook_config` inside the conversation request. They operate inside the agent loop, not in the workspace-lifecycle layer.

The MVP does not require server-side OpenHands hooks to succeed. They are an advanced extension.

## 11. MCP tool strategy

Prefer MCP over custom Python-only server-side tools whenever possible.

Why:

- transport-neutral
- works well from a Rust orchestrator
- avoids building and maintaining a custom Python runtime image too early
- cleaner path to future hosted mode

For local MVP, the planned MCP surface is a small Linear tool server launched via stdio.

## 12. Hosted-mode implications kept in mind during MVP

The local MVP chooses a single local server process, but the OpenHands integration boundary must also support future remote deployment.

Future hosted mode changes:

- `base_url` points to a remote agent-server fleet
- auth becomes mandatory
- local subprocess supervisor is disabled
- stronger sandbox and tenancy rules apply

What does not change:

- conversation REST contract
- WebSocket runtime stream contract
- issue workspace ownership at the Symphony layer
- control-plane API shape for the UI

## 13. Failure and fallback policy

If the agent-server is unavailable:

- log a transport-layer failure with issue context
- fail the current worker
- schedule a Symphony retry with backoff
- keep the daemon alive

If a conversation is unrecoverable:

- mark the persisted conversation as invalid
- optionally archive its metadata
- create a fresh conversation on the next worker attempt if policy allows

## 14. Implementation rules for coding agents

- Build against a pinned OpenHands version.
- Mirror only the required REST schema subset.
- Keep raw payload logging for debugging behind redaction.
- Do not pull web-app endpoints into the runtime crate.
- Do not assume any undocumented endpoint exists without a pinned-version integration test.
