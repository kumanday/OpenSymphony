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
opensymphony run
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
  `tools/openhands-server/{pyproject.toml,version.txt,uv.lock,install.sh,run-local.sh}`
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

- `tools/openhands-server/pyproject.toml` pins the local server environment, `tools/openhands-server/install.sh` performs the locked `uv sync --extra agent-server` bootstrap, and `tools/openhands-server/run-local.sh` starts it via `uv`
- `opensymphony-openhands` currently implements the typed conversation create, get, send-message, run, paginated event search, readiness probe, and `RuntimeEventStream` attach/reconcile/reconnect surface used by validation and doctor flows
- workflow resolution now forwards `openhands.conversation.agent.tools` and optional `openhands.conversation.agent.include_default_tools` into the typed OpenHands conversation-create payload; partial agent overrides still inherit the default LLM model unless the workflow explicitly replaces it, and when the tool fields are omitted OpenSymphony adds `TerminalTool` and `FileEditorTool` as the default coding-agent tools while leaving the agent-server default `FinishTool` and `ThinkTool` set under OpenHands control
- `opensymphony-testkit` emulates the same endpoint subset for deterministic CI coverage and now supports scripted `/events/search` responses plus per-connection WebSocket frame sequences so attach/reconcile race windows, buffered live events, and reconnect drops can be reproduced without bespoke inline servers
- `opensymphony doctor` now resolves the target repo `WORKFLOW.md` before probing OpenHands, so the live probe uses workflow-derived workspace, transport, conversation, and prompt inputs instead of only static CLI YAML fields
- `opensymphony doctor` now checks for `cargo`, `curl`, `git`, and `uv` on `PATH`, prints the trusted-machine local-safety warning on every run, and warns when a local deployment points at a non-loopback OpenHands target
- `tools/openhands-server/run-local.sh` resolves its own directory before invoking `uv`, enforces `uv run --directory <tool-dir> --locked --extra agent-server --module openhands.agent_server`, and rejects extra agent-server CLI flags so the pinned project works the same way from the repo root, CI, and the local supervisor
- when `openhands.local_server.command` is omitted, workflow resolution leaves the field unset and the runtime-owned local tooling layer resolves the pinned `tools/openhands-server/run-local.sh` launcher from the OpenSymphony checkout before the supervisor switches `cwd` to the issue workspace, even when the workflow itself lives in a separate target repo
- explicit `openhands.local_server.command` overrides are currently rejected during workflow resolution until the runtime supervisor can honor workflow-owned launcher commands instead of always starting the pinned repo-local launcher
- explicit `openhands.local_server.enabled: false` overrides are currently rejected during workflow resolution until the runtime supervisor can honor workflow-owned local-server disablement instead of still deciding launch behavior from the localhost base URL plus pinned tooling readiness
- explicit `openhands.local_server.env` overrides are currently rejected during workflow resolution until the runtime supervisor creation path forwards workflow-owned launcher environment variables into `extra_env`
- explicit `openhands.local_server.startup_timeout_ms` overrides are currently rejected during workflow resolution until the runtime supervisor creation path consumes workflow-owned startup timeout settings instead of always using the supervisor default
- workflow resolution now accepts absolute `http://` and `https://` `openhands.transport.base_url` values with optional path prefixes, rejects embedded credentials plus query/fragment suffixes, still rejects bracketed IPv6 until the local readiness probe supports it, and requires `https://` plus `openhands.transport.session_api_key_env` for non-loopback targets

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
- the supervisor sets `OPENHANDS_SERVER_PORT`, while the launcher itself forces `RUNTIME=process`, loopback host `127.0.0.1`, the pinned `agent-server` extra, and `uv` lockfile enforcement
- before spawning, supervised mode probes the resolved base URL and fails fast if another ready server is already responding there, so the daemon never treats a foreign process as its owned child
- diagnostics record the launcher summary, resolved base URL, pinned version,
  and launched PID for doctor output and future daemon logs
- the doctor path renders the target repo workflow prompt with a synthetic issue
  and sends that rendered prompt inside the probe message so prompt/template
  regressions fail before a real issue runner lands
- the current doctor and live-validation path uses `GET /openapi.json` as the
  conservative readiness probe and will temporarily start a supervised local
  server only when the configured target is an unauthenticated loopback
  `http://` root origin and the repo-owned pin is valid; authenticated,
  path-prefixed, or non-loopback targets stay in external mode and are probed in
  place
- live-only doctor overrides such as `probe_model` and `probe_api_key_env` are
  resolved lazily when `--live-openhands` is requested, so shared configs can
  leave those `${VAR}` placeholders unset during the static preflight path

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

The runtime adapter must also surface progress heartbeats from the WebSocket event stream back to the orchestrator so stall detection is based on time since the last observed runtime event, not just time since the worker launched.

## 6. Conversation model

## 6.1 Stable conversation identity

Each issue gets a stable `conversation_id` persisted under:

```text
<issue_workspace>/.opensymphony/conversation.json
```

Suggested persisted fields:

- `issue_id`
- `identifier`
- `conversation_id`
- `created_at`
- `updated_at`
- `last_attached_at`
- `launch_profile`
- `server_base_url`
- `transport_target`
- `http_auth_mode`
- `websocket_auth_mode`
- `websocket_query_param_name`
- `persistence_dir`
- `fresh_conversation`
- `workflow_prompt_seeded`
- `reset_reason`
- `runtime_contract_version`
- `last_prompt_kind`
- `last_prompt_path`
- `last_execution_status`
- `last_event_id`
- `last_event_kind`
- `last_event_at`
- `last_event_summary`

`launch_profile` should capture the OpenHands conversation settings that need to survive restarts and interactive debug reuse, including:

- `workspace_kind`
- `confirmation_policy_kind`
- `agent_kind`
- `llm_model`
- `agent_tools`
- `agent_include_default_tools`
- `max_iterations`
- `stuck_detection`

Implementation note:

- `opensymphony-workspace` owns the deterministic `conversation.json` path and serialization helpers
- `opensymphony-openhands::IssueSessionRunner` decides when to create, reuse, attach, or reset the conversation and populates the runtime-facing fields above

## 6.2 Persistence directory

Each issue conversation should persist under a stable path inside the issue workspace, for example:

```text
<issue_workspace>/.opensymphony/openhands/
```

This keeps execution state co-located with the workspace and simplifies recovery.

Current implementation detail:

- the adapter derives `persistence_dir` from `openhands.conversation.persistence_dir_relative`
  joined under the sanitized issue workspace root
- conversation reuse checks compare the persisted manifest path against that resolved
  workflow-owned directory instead of hard-coding `.opensymphony/openhands`

## 6.3 Reuse policy

Default policy:

- reuse the conversation for the same issue across worker lifetimes
- when reusing a conversation, send continuation-only guidance instead of replaying the full assignment body
- if a run fails after attach, preserve the known `conversation_id` in workspace metadata so the next retry can resume the same conversation instead of forcing a fresh thread
- if persisted conversation metadata is invalid locally, clear it and treat the next dispatch as a fresh reset instead of retrying the corrupt manifest forever
- reset only when:
  - conversation metadata is missing or invalid
  - the server reports the conversation cannot be attached
  - an incompatible protocol version is detected
  - an explicit reset policy is configured

Current implementation detail:

- `opensymphony-openhands::IssueSessionRunner` owns `conversation.json`
- fresh conversations start with `workflow_prompt_seeded = false`
- the full workflow prompt is selected until a `POST /events` call accepts that first assignment message
- once seeded, later worker lifetimes send built-in continuation guidance instead of rerendering the workflow template
- if `GET /api/conversations/{id}` or the initial attach fails for a reused conversation, the runner retries `POST /api/conversations` with the same stable `conversation_id`; when that re-created thread still exposes persisted history, the runner keeps continuation guidance instead of downgrading to a fresh full prompt
- the runner persists the conversation launch profile on first create and backfills older manifests on reuse so later rehydration and interactive debug sessions can recreate the same thread settings, including agent tool selection, without guessing from mutable runtime state

## 6.4 Interactive debug resumption

`opensymphony debug <issue-id>` resolves the managed workspace for an issue, reads
`conversation.json`, and attaches to the recorded `conversation_id` from the original
working directory.

Current implementation detail:

- external configured transports are reused as-is
- local-supervised workflows probe the configured base URL first and reuse any ready server already responding there
- if no ready local server exists, the debug command can launch the pinned repo-local supervisor and then reattach
- if the server no longer has the conversation but the persisted history directory still exists, the CLI recreates the same `conversation_id` using the stored `launch_profile` and then resumes the thread
- operators should avoid unrelated standalone `openhands` CLI sessions on the same port so the orchestrator-managed transport remains the single source of truth
- if a reused conversation is already active, the runner waits for that turn to finish before sending the next prompt, and it retries `POST /run` after a `409 Conflict` on the same attached stream
- each fresh create also snapshots `create-conversation-request.json`, `last-conversation-state.json`, and `generated/session-context.json` inside the issue workspace for recovery and observability

## 6.4 Scheduler handoff contract

The orchestrator does not consume raw OpenHands protocol frames directly.

The scheduler-facing worker boundary should provide:

- a launch acknowledgment that includes `ConversationMetadata` so the issue can move into `Running`
- incremental runtime-event reports used to refresh last-event timestamps and stall deadlines
- one terminal worker outcome report used to schedule continuation retry, failure backoff, or release

Current repository implementation:

- `opensymphony-openhands::IssueSessionRunner` still owns the concrete attach/create/send/run/await flow
- `opensymphony-orchestrator::Scheduler` is now implemented as a generic core over a worker backend that emits the launch plus runtime/outcome reports above
- fake-backend scheduler tests currently lock down the orchestration semantics while a thin production adapter from `IssueSessionRunner` into that worker contract remains follow-on wiring

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
- non-default `openhands.conversation.reuse_policy` values are rejected during workflow resolution until the orchestrator/runtime path can honor alternate conversation reuse behavior
- `max_iterations` must fit the downstream OpenHands `u32` request range
- `openhands.transport.session_api_key_env` is accepted and required for non-loopback remote targets
- workflow-owned `local_server.readiness_probe_path` overrides are rejected during workflow resolution until the runtime supervisor launch path consumes them
- workflow-owned `local_server.startup_timeout_ms` overrides are rejected during workflow resolution until the runtime supervisor creation path consumes them
- `openhands.websocket.auth_mode` defaults to `auto` and `openhands.websocket.query_param_name` defaults to `session_api_key`
- workflow-owned `websocket.enabled` overrides are rejected during workflow resolution until the runtime readiness path can honor disabling the socket entirely
- workflow-owned `websocket.ready_timeout_ms`, `websocket.reconnect_initial_ms`, and `websocket.reconnect_max_ms` overrides now resolve into the runtime stream attach and reconnect budgets
- `agent.llm.model` is required whenever an `llm` block is present
- workflow-owned LLM option keys are rejected during workflow resolution until the current request subset can actually forward them
- workflow-owned agent options such as `log_completions` and extra agent keys are rejected during workflow resolution until the current request subset can actually forward them
- workflow-owned LLM provider env overrides such as `api_key_env` and `base_url_env` are rejected during workflow resolution until the runtime conversation-create adapter can actually forward them
- workflow-owned `openhands.mcp.stdio_servers` entries are rejected during workflow resolution until the runtime conversation-create adapter can actually send `mcp_config`

Implementation rule:

- keep the orchestrator core independent of the raw OpenHands JSON model
- all payload shaping belongs in `opensymphony-openhands`

Current repository implementation:

- `ConversationCreateRequest` carries the minimal create payload subset, including `conversation_id`, `workspace.working_dir`, and `persistence_dir`
- the current request model still serializes `agent` as only `{ kind, llm }`, and `llm` itself as only `{ model, api_key }`, so workflow-owned agent extras plus arbitrary LLM option keys are rejected before runtime launch
- the current orchestrator/runtime path still uses fixed per-issue conversation reuse, so workflow-owned `reuse_policy` overrides are rejected before runtime launch
- the current transport layer preserves base-path prefixes across REST endpoints and `/sockets/events/{conversation_id}`, so the same client can target reverse-proxied external servers without code changes outside config
- the current supervisor readiness probe still owns the local launch path and always uses `/openapi.json`, so explicit `local_server.readiness_probe_path` and `local_server.startup_timeout_ms` overrides are still rejected before runtime launch
- the current supervisor launch path still uses runtime-owned launcher environment variables (`OPENHANDS_SERVER_PORT` and `RUNTIME=process`), so explicit workflow-owned `local_server.env` overrides are rejected before runtime launch
- the current runtime now consumes workflow-owned `websocket.ready_timeout_ms`, `websocket.reconnect_initial_ms`, and `websocket.reconnect_max_ms` values, but still always opens the readiness socket so explicit `websocket.enabled` overrides remain rejected before runtime launch
- the current request model does not yet serialize `mcp_config`, so workflow-owned MCP stdio server declarations are rejected before runtime launch
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

- `TransportConfig::from_workflow` now resolves `openhands.transport.session_api_key_env`, `openhands.websocket.auth_mode`, and `openhands.websocket.query_param_name` into `AuthConfig`
- when a session API key is configured, REST always uses the pinned `x-session-api-key` header shape while WebSocket auth follows the workflow mode: `auto` and `query_param` use the configured query parameter name, and `header` sends the same header on the socket handshake
- REST auth is applied independently from WebSocket auth so remote/header deployments do not force the local query-param shape, while the default `auto` mode matches the pinned 1.14.0 SDK behavior
- non-loopback remote targets must use `https://` and provide `openhands.transport.session_api_key_env` during workflow resolution
- `IssueSessionRunner` persists `server_base_url`, `transport_target`, `http_auth_mode`, `websocket_auth_mode`, and `websocket_query_param_name` into `conversation.json`, generated session context, and control-plane snapshots for remote troubleshooting
- `OpenHandsError` now maps invalid config, transport failures, HTTP status failures, protocol failures, and WebSocket failures into stable runtime categories without exposing `reqwest::Error` or `http::StatusCode`
- `crates/opensymphony-openhands/tests/client_resilience.rs`, `crates/opensymphony-openhands/tests/transport_config.rs`, `crates/opensymphony-openhands/tests/supervisor.rs`, and the opt-in `crates/opensymphony-openhands/tests/live_pinned_server.rs` suite cover authenticated REST operations, WebSocket readiness auth, auth failure mapping, path-prefixed external targets, local-supervisor eligibility rules, external-server no-op stop behavior, and pinned-server HTTP/WebSocket auth success and failure paths
- the doctor probe now runs through `RuntimeEventStream`, exercises a real `POST /events` plus `POST /run` path, and only reports the runtime healthy after the attached stream reaches a successful terminal `execution_status` of `finished` with no queued `ConversationErrorEvent` still pending ahead of completion or arriving on the next scheduler-turn buffered drain
- failure-only probe streams such as `ConversationErrorEvent` or terminal `execution_status` values like `error` and `stuck` are treated as unhealthy instead of silently passing
- once the attached stream has already observed a healthy terminal outcome, the doctor probe reuses the last successful stream-backed conversation snapshot instead of requiring one more `GET /api/conversations/{id}` that could fail during server shutdown
- readiness snapshots are attach/reconnect barriers keyed by envelope kind, not synthetic replay events; consumers observe them through `ready_event`, and the adapter folds them into `state_mirror()` only when reconcile and REST refresh do not already expose an equal or newer decodable state update, including forward-compatible payloads that still carry a usable `state_delta`; that barrier state is also re-applied after later cache-driven mirror rebuilds so stale queued snapshots do not override a newer ready barrier, and an active `queued` or `running` ready barrier may also clear stale terminal REST fallback when a reused conversation has already restarted
- initial attach now replays the persisted `/events/search` snapshot through `RuntimeEventStream::next_event()` so resumed conversations expose pre-existing history in timestamp order, while any immediately available live socket frames are merged into that same ordered queue before later replay items are yielded instead of relying on a fixed drain delay
- if the socket closes after already-yieldable events have been drained into the pending queue, reconnect is deferred until that queued work has been delivered
- `RuntimeEventStream::close()` is terminal for that stream instance: it clears deferred reconnect intent and queued replay before closing the socket so later polls do not reopen the conversation
- the doctor probe accepts a terminal REST refresh after disconnect as authoritative completion evidence even if the follow-on WebSocket reattach exhausts

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

For local MVP, the implemented MCP surface is a small Linear tool server launched via stdio:

- `opensymphony linear-mcp`
- line-delimited JSON-RPC over stdio
- tool set:
  - `linear_get_issue`
  - `linear_comment_issue`
  - `linear_transition_issue`
  - `linear_link_pr`
  - `linear_list_project_states`

`WORKFLOW.example.md` does not yet declare `mcp.stdio_servers`: workflow-owned
MCP config remains rejected until the conversation-create adapter can actually
forward `mcp_config` to OpenHands, so local sessions must provision
`opensymphony linear-mcp` through the host tool environment for now.

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
