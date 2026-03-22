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
       └─ RUNTIME=process python -m openhands.agent_server --host 127.0.0.1 --port 8000
```

Key properties:

- one local OpenHands server process per daemon
- many issue-specific workspaces via conversation `working_dir`
- no per-issue Docker in the MVP
- host-local execution assumptions documented explicitly
- the local supervised launch path forces OpenHands process-sandbox mode via
  `RUNTIME=process`
- the current implementation resolves supervised-mode metadata from
  `tools/openhands-server/{pyproject.toml,version.txt,uv.lock,run-local.sh}`
  with a repo-owned package and lockfile pin
- the repo-local quick-run wrapper rejects user-supplied agent-server CLI flags
  so smoke runs cannot diverge from the daemon-managed single-server topology

## 4. Server lifecycle model

## 4.1 Modes

OpenSymphony supports two runtime modes through the same trait boundary.

### Local supervised mode

The daemon launches and manages one local OpenHands agent-server subprocess.

Use for:

- single developer setup
- local experimentation
- CI smoke environments with trusted repos

Repository ownership note:

- `tools/openhands-server/` owns the local packaging and version pin
- the current repository pin is the OpenHands `1.14.0` SDK bundle
- the lockfile is resolved under the repo-local `uv` environment
- update `docs/sources.md` whenever this version pin changes

### External server mode

The daemon connects to a pre-existing agent-server at `openhands.transport.base_url`.

Use for:

- integration tests against a pinned external server
- future hosted deployment mode
- organization-managed runtime infrastructure

Local MVP work starts with supervised mode, but the trait boundary must support both.

Current repository implementation:

- `tools/openhands-server/pyproject.toml` pins the local server environment and `tools/openhands-server/run-local.sh` starts it via `uv`
- `opensymphony-openhands` currently implements the minimal typed conversation create, get, send-message, run, search, and WebSocket readiness probe surface used by validation and doctor flows
- `opensymphony-testkit` emulates the same endpoint subset for deterministic CI coverage
- `tools/openhands-server/run-local.sh` resolves its own directory before invoking `uv` so the pinned project works even when the caller runs it from the repo root
- when `openhands.local_server.command` is omitted, workflow resolution leaves the field unset and the runtime-owned local tooling layer resolves the pinned `tools/openhands-server/run-local.sh` launcher from the OpenSymphony checkout before the supervisor switches `cwd` to the issue workspace, even when the workflow itself lives in a separate target repo
- explicit `openhands.local_server.command` overrides are currently rejected during workflow resolution until the runtime supervisor can honor workflow-owned launcher commands instead of always starting the pinned repo-local launcher
- workflow resolution rejects malformed, non-HTTP(S), or `/api`-suffixed `openhands.transport.base_url` values before the daemon reaches runtime transport setup

## 4.2 Startup contract

In supervised mode:

1. Resolve the configured command and environment.
2. Start the subprocess bound to `127.0.0.1` with `RUNTIME=process` so the
   local MVP stays on the documented host-process execution path.
3. Wait for readiness by probing a known HTTP endpoint.
4. Record server metadata in memory and logs.
5. Reuse the server for all issue runs.

Readiness probing rule:

- prefer a documented health endpoint if the pinned version exposes one
- otherwise use a conservative FastAPI probe such as `GET /openapi.json`
- never rely on sleep-only startup delays

Current implementation detail:

- supervised mode launches `bash tools/openhands-server/run-local.sh`
- the supervisor sets `OPENHANDS_SERVER_PORT` and `RUNTIME=process` explicitly
- diagnostics record the launcher summary, resolved base URL, pinned version,
  and launched PID for doctor output and future daemon logs
- the current doctor and live-validation path uses `GET /openapi.json` as the
  conservative readiness probe and will temporarily start a supervised local
  server when the configured loopback base URL is down but the repo-owned pin is
  valid

## 4.3 Shutdown contract

On daemon shutdown:

- stop creating new issue workers
- cancel active workers cleanly
- close live WebSocket clients
- stop the supervised agent-server subprocess if this daemon launched it

If the daemon is using an external server, never attempt to terminate that server.

Current implementation detail:

- child ownership is tracked by the Rust supervisor instance
- `stop()` only kills a `Child` handle created by `start()`
- external mode may probe health, but stop remains a no-op

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

The current fake-server coverage and CLI doctor implementation exercise this exact subset.

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

Current workflow defaulting:

- `confirmation_policy.kind` defaults to `NeverConfirm` when omitted
- unsupported `confirmation_policy` options are rejected during workflow resolution because the current request subset only serializes `{ kind }`
- `agent.kind` defaults to `Agent` when omitted
- `max_iterations` must fit the downstream OpenHands `u32` request range
- `agent.llm.model` is required whenever an `llm` block is present
- workflow-owned LLM provider env overrides such as `api_key_env` and `base_url_env` are rejected during workflow resolution until the runtime conversation-create adapter can actually forward them

Implementation rule:

- keep the orchestrator core independent of the raw OpenHands JSON model
- all payload shaping belongs in `opensymphony-openhands`

Current repository implementation:

- `ConversationCreateRequest` carries the minimal create payload subset, including `conversation_id`, `workspace.working_dir`, and `persistence_dir`
- `ConversationRunRequest` serializes the empty `{}` body used by `POST /api/conversations/{conversation_id}/run`
- `AcceptedResponse` tolerates either an explicit JSON success body or an empty successful response for `POST /events` and `POST /run`

## 9. Authentication

## 9.1 Local MVP

Default local mode:

- bind to loopback only
- no mandatory session API key
- the current repository pin is the OpenHands `1.14.0` SDK bundle

## 9.2 Future-proofing

The Rust client should still support:

- no auth
- session API key for HTTP
- session API key for WebSocket query-param fallback
- optional header-based WebSocket auth for versions that support it

Current repository implementation:

- `TransportConfig` now carries an `AuthConfig` with explicit no-auth, query-param API key, header API key, and header-plus-WebSocket-query-fallback modes
- REST auth is applied independently from WebSocket auth so remote/header deployments do not force the local query-param shape
- workflow-owned auth knobs such as `openhands.transport.session_api_key_env`, `openhands.websocket.auth_mode`, and `openhands.websocket.query_param_name` are currently rejected during workflow resolution until a runtime adapter wires them into `AuthConfig`
- `OpenHandsError` now maps invalid config, transport failures, HTTP status failures, protocol failures, and WebSocket failures into stable runtime categories without exposing `reqwest::Error` or `http::StatusCode`
- `crates/opensymphony-openhands/tests/client_resilience.rs` covers authenticated REST operations, WebSocket readiness auth, auth failure mapping, malformed payload handling, and non-readiness frames before the first state update
- the doctor probe now exercises a real `POST /events` plus `POST /run` path and only reports the runtime healthy after a successful terminal `execution_status` of `finished`
- failure-only probe streams such as `ConversationErrorEvent` or terminal `execution_status` values like `error` and `stuck` are treated as unhealthy instead of silently passing

Do not assume one auth method forever. Make it configurable and covered by integration tests against the pinned version.

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
