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
       └─ python -m openhands.agent_server --host 127.0.0.1 --port 8000
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

Use for:

- integration tests against a pinned external server
- future hosted deployment mode
- organization-managed runtime infrastructure

Local MVP work starts with supervised mode, but the trait boundary must support both.

## 4.2 Startup contract

In supervised mode:

1. Resolve the configured command and environment.
2. Start the subprocess bound to `127.0.0.1`.
3. Wait for readiness by probing a known HTTP endpoint.
4. Record server metadata in memory and logs.
5. Reuse the server for all issue runs.

Readiness probing rule:

- prefer a documented health endpoint if the pinned version exposes one
- otherwise use a conservative FastAPI probe such as `GET /openapi.json`
- never rely on sleep-only startup delays

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

Each issue conversation should persist under a stable path inside the issue workspace, for example:

```text
<issue_workspace>/.opensymphony/openhands/
```

This keeps execution state co-located with the workspace and simplifies recovery.

## 6.3 Reuse policy

Default policy:

- reuse the conversation for the same issue across worker lifetimes
- reset only when:
  - conversation metadata is missing or invalid
  - the server reports the conversation cannot be attached
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
  - `kind`
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

Implementation rule:

- keep the orchestrator core independent of the raw OpenHands JSON model
- all payload shaping belongs in `opensymphony-openhands`

## 9. Authentication and transport profile

The auth surface should hang off the existing `WORKFLOW.md` OpenHands config instead of introducing a second hosted-only config path.

Relevant fields:

- `openhands.transport.base_url`
- `openhands.transport.session_api_key_env`
- `openhands.local_server.enabled`
- `openhands.websocket.auth_mode`
- `openhands.websocket.query_param_name`

## 9.1 Local MVP

Default local mode:

- `base_url` points at loopback
- `local_server.enabled` is `true`
- bind to loopback only
- no mandatory session API key

## 9.2 External or hosted server mode

When the daemon targets an externally managed or hosted server:

- `base_url` must be explicit
- `local_server.enabled` must be `false`
- hosted deployments should require `https://`
- `session_api_key_env` should resolve at startup
- `websocket.auth_mode` must be pinned or validated against the selected OpenHands version

## 9.3 Supported auth strategies

The Rust client should still support:

- no auth
- session API key for HTTP
- session API key for WebSocket query-param fallback
- optional header-based WebSocket auth for versions that support it

Do not assume one auth method forever. Keep it configurable, prefer the most secure strategy the pinned version supports, and cover the exact HTTP plus WebSocket behavior with integration tests.

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

Hosted troubleshooting expectations:

- expose the remote base URL host and scheme through structured logs or the control plane
- expose whether `local_server.enabled` is on or off
- expose the selected WebSocket auth mode and readiness timeout
- record the pinned server version when it can be discovered safely
- never expose session API keys, raw auth headers, or raw query parameters

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
