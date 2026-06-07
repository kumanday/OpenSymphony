# OpenSymphony Host-Client Architecture

## 1. Architecture goals

The architecture must support a rich local desktop app, a browser-based web app, and a hosted server mode while preserving OpenSymphony's core orchestration guarantees.

The goals are:

1. Keep OpenSymphony's Rust orchestrator authoritative for scheduling, workspace state, harness state, retries, reconciliation, and run outcomes.
2. Expose a versioned OpenSymphony Gateway for all rich clients.
3. Share most frontend code between desktop and web.
4. Support high-throughput streams for runtime events, terminal output, logs, diffs, approvals, and planning sessions.
5. Keep OpenHands agent-server as the initial harness integration through server-owned HTTP and WebSocket clients.
6. Prepare harness abstractions for future Codex app-server support.
7. Prepare authentication and model abstractions for OpenAI ChatGPT/Codex subscription access independent of any single harness.
8. Support hosted execution where long-running work continues after clients disconnect.
9. Support Linear-backed task graph management and collaborative spec-driven project planning.
10. Make UI failures non-fatal to orchestration correctness.

## 2. Core principles

### 2.1 The orchestrator is authoritative

Scheduling state, run state, workspace state, retry state, and reconciliation state live on the OpenSymphony host. Clients issue versioned actions and render state. They do not directly mutate private orchestrator internals.

### 2.2 Runtime attachment is server-owned

Clients do not attach directly to OpenHands agent-server or future Codex app-server sessions during normal operation. The OpenSymphony host owns harness attachment, event decoding, reconciliation, event caching, and normalization. Clients consume OpenSymphony events.

### 2.3 REST establishes state, streams keep it live

The gateway uses REST-style endpoints for initial snapshots, detail reads, mutations, recovery, and reconciliation. It uses streaming channels for live updates and high-volume runtime data.

### 2.4 Shared frontend, separate shells and transports

The desktop and web clients share UI components, state reducers, schemas, and contract-level transport semantics. They do not need the same physical transport. Desktop-only capabilities are isolated behind a Tauri adapter that can use native local channels when OpenSymphony is running locally. Web-only capabilities are isolated behind a browser adapter that uses authenticated network transports.

### 2.5 Hosted mode is a topology change

Hosted mode changes authentication, tenancy, workspace isolation, secrets, resource limits, and deployment. It should not require rewriting the orchestrator, task graph, harness adapters, or client state model.

### 2.6 Future integrations need seams now

Codex app-server, subscription credential support, API-compatible model settings, and additional trackers are future capabilities. The initial implementation should include the adapter seams, schema design, and settings model needed to add them without a major rewrite.

## 3. Deployment topologies

### 3.1 Local rich desktop mode

```text
Developer machine

Tauri desktop app
├─ Webview frontend
│  ├─ Project dashboard
│  ├─ Task graph
│  ├─ Run details
│  ├─ Terminal/log/diff panes
│  └─ Planning workspace
├─ Rust desktop shell
│  ├─ Native settings
│  ├─ OS keychain
│  ├─ Local daemon supervisor, optional
│  └─ Tauri command/channel bridge
└─ OpenSymphony daemon
   ├─ Gateway API
   ├─ Orchestrator
   ├─ Linear task graph service
   ├─ Workspace manager
   ├─ Harness manager
   ├─ Event journal
   └─ OpenHands agent-server supervisor
```

In this mode, the desktop app may supervise the local daemon, connect to a daemon started separately, or attach to an embedded/local host if that packaging model is selected. Heavy execution runs on the developer machine. The desktop app gets the best local integration and lowest latency.

The preferred local transport order is:

1. In-process Rust channels when the OpenSymphony host is embedded in the Tauri backend.
2. Native local IPC such as Unix domain sockets on macOS/Linux or named pipes on Windows when the host is a separate local process.
3. Tauri channels from the Rust backend into the webview for high-volume frames.
4. Loopback HTTP/WebSocket as the compatibility path and as the baseline contract test path.

All four paths must expose the same gateway DTOs, event envelopes, cursors, and action receipts. Local native transport is a performance optimization; it must not bypass the event journal, permission checks, or orchestrator-owned state transitions.

### 3.2 External local server mode

```text
Developer machine or trusted local network

Desktop client or web client
└─ HTTP/WebSocket
   └─ OpenSymphony Gateway
      ├─ Orchestrator
      ├─ Workspace manager
      └─ External OpenHands agent-server
```

This mode supports debugging, CI, protocol testing, and early separation of client and host. It is also the bridge toward hosted mode.

### 3.3 Hosted remote mode

```text
Users
├─ Desktop client
└─ Web client

HTTPS/WSS
└─ Hosted OpenSymphony Gateway
   ├─ Auth and RBAC
   ├─ Organization and tenant store
   ├─ Orchestrator service
   ├─ Task graph service
   ├─ Planning service
   ├─ Harness manager
   ├─ OpenHands runtime fleet
   ├─ Future Codex app-server runtime support
   ├─ Workspace isolation layer
   ├─ Event journal and stream broker
   ├─ Artifact and log storage
   ├─ Secret store
   └─ Metrics and audit logs
```

In this mode, execution continues without connected clients. Users can access the same server through desktop or web. The web client may be served by the hosted gateway or deployed as a separate static frontend.

The desktop app uses a remote gateway profile in hosted mode. It should not attempt to tunnel local native APIs to a hosted server. Remote desktop and browser clients should share the same authenticated HTTPS/WSS semantics, including stream cursors, replay, action correlation IDs, and hosted authorization checks.

## 4. Server-side architecture

### 4.1 OpenSymphony Gateway

The gateway is the client-facing API layer. It exposes versioned APIs and converts user intent into orchestrator actions.

Responsibilities:

- Health and readiness.
- Capability discovery.
- Project and repository APIs.
- Task graph APIs.
- Run and workspace APIs.
- Event stream APIs.
- Terminal/log/diff APIs.
- Planning-session APIs.
- Mutation/action APIs.
- Auth/RBAC enforcement in hosted mode.
- Correlation IDs for actions and events.
- API versioning and compatibility.

The gateway should be implemented as a Rust service layer inside the OpenSymphony host. It may use the existing control plane as the starting point, but it should evolve from read-only snapshots into an intent-based control API.

### 4.2 Orchestrator service

The orchestrator remains the scheduling source of truth.

Responsibilities:

- Poll Linear or receive tracker updates where supported.
- Determine issue eligibility.
- Enforce hierarchy-aware scheduling.
- Manage bounded concurrency.
- Create run records.
- Ask the workspace manager for workspaces.
- Ask the harness manager to start or resume execution.
- Track run lifecycle state.
- Schedule retries.
- Detect stalls.
- Reconcile tracker state and runtime state.
- Persist authoritative state.

The orchestrator publishes state changes to the event journal. It does not depend on UI availability.

### 4.3 Workspace manager

The workspace manager owns logical and physical workspaces.

Responsibilities:

- Derive stable workspace keys.
- Create per-issue or per-sub-issue workspaces.
- Run lifecycle hooks.
- Track repository paths, branches, worktrees, and cleanup metadata.
- Provide logical workspace IDs to hosted mode.
- Abstract local paths from hosted containers, VMs, or managed sandboxes.
- Expose safe file and diff summaries through the gateway.

Hosted mode should treat workspace paths as logical identifiers. Physical paths must remain inside the workspace isolation layer.

### 4.4 Harness manager

The harness manager provides a common interface for agent execution substrates.

```rust
trait HarnessAdapter {
    fn harness_kind(&self) -> HarnessKind;
    fn capabilities(&self) -> HarnessCapabilities;
    async fn create_or_resume_session(&self, req: SessionRequest) -> Result<SessionHandle>;
    async fn start_run(&self, session: SessionHandle, req: RunRequest) -> Result<RunHandle>;
    async fn send_user_message(&self, session: SessionHandle, msg: UserMessage) -> Result<()>;
    async fn cancel_run(&self, run: RunHandle) -> Result<ActionResult>;
    async fn pause_run(&self, run: RunHandle) -> Result<ActionResult>;
    async fn resume_run(&self, run: RunHandle) -> Result<ActionResult>;
    async fn attach_events(&self, session: SessionHandle, cursor: Option<EventCursor>) -> Result<EventStream>;
    async fn fetch_history(&self, session: SessionHandle, cursor: Option<EventCursor>) -> Result<Vec<HarnessEvent>>;
}
```

The exact trait should be adapted to the Rust codebase, but the conceptual boundary should stay stable.

#### OpenHands adapter

Initial production adapter.

Responsibilities:

- Create conversations through HTTP.
- Send messages and trigger runs through HTTP.
- Search event history through HTTP.
- Attach to WebSocket events.
- Preserve the initial REST sync, WebSocket readiness barrier, and post-ready reconciliation sequence.
- Decode high-value OpenHands events.
- Preserve unknown events as raw JSON.
- Normalize events into OpenSymphony event envelopes.
- Maintain event cache and state mirror.

#### Codex app-server adapter, future

Future adapter.

Responsibilities:

- Launch or connect to `codex app-server`.
- Use JSON-RPC request/response and notification handling.
- Prefer stdio transport for local integration.
- Treat WebSocket transport as experimental until benchmarked and secured.
- Generate schema artifacts for the installed Codex version where supported.
- Normalize thread, turn, message, tool, approval, and completion events.
- Surface Codex approvals through the OpenSymphony approval center.
- Respect selected model settings and harness capability.

#### Pi or Rust-native adapter, future

Future adapter where in-process Rust execution or Rust SDK embedding is justified.

Responsibilities:

- Provide a high-performance local harness path.
- Use an in-process SDK where stable.
- Fall back to subprocess/RPC if needed.
- Normalize events into the same OpenSymphony run schema.

### 4.5 Task graph service

The task graph service joins Linear state and OpenSymphony runtime state.

Responsibilities:

- Query Linear projects, milestones, issues, sub-issues, relations, comments, labels, and statuses.
- Cache tracker data with sync timestamps.
- Join tracker nodes with runtime overlays.
- Expose project, milestone, issue, and sub-issue views.
- Validate Linear mutations.
- Execute Linear GraphQL mutations through a supported service or checked-in helper/query assets.
- Provide dependency and blocker views.
- Preserve mapping between Linear IDs and OpenSymphony entity IDs.

Data model:

```text
Project
├─ Milestone, Linear project milestone
│  ├─ Issue, Linear issue
│  │  ├─ SubIssue, Linear sub-issue
│  │  └─ Runtime overlays
│  └─ Milestone metrics
└─ Project metrics
```

Runtime overlays:

```text
RuntimeOverlay
├─ eligibility_state
├─ queue_state
├─ active_run_id
├─ last_outcome
├─ retry_count
├─ workspace_id
├─ harness_session_id
├─ last_event_seq
├─ diff_summary
├─ validation_summary
├─ token_cost_summary
└─ blocker_summary
```

### 4.6 Planning service

The planning service supports collaborative spec-driven project kickoff. It adapts GSD-2's task-creation workflow through Linear-native project, milestone, issue, and sub-issue language.

Responsibilities:

- Maintain planning sessions.
- Store conversation turns.
- Store structured artifacts.
- Analyze repository and existing Linear state.
- Generate milestone, issue, and sub-issue drafts.
- Track assumptions, requirements, risks, dependencies, and acceptance criteria.
- Research public documentation, APIs, ecosystem references, and other relevant external sources.
- Analyze codebase structure, conventions, ownership boundaries, and integration points.
- Generate a dependency graph across milestones, issues, and sub-issues.
- Run plan quality checks before publishing.
- Produce draft Linear mutation payloads.
- Require human approval before publishing.
- Publish to Linear through the task graph service.
- Convert approved plan items into OpenSymphony-ready task graph entities.

Planning artifacts:

```text
PlanningSession
├─ Intake
├─ ProjectContext
├─ Requirements
├─ ResearchBrief
├─ CodebaseAnalysis
├─ ArchitectureNotes
├─ RiskRegister
├─ MilestonePlan
├─ IssuePlan
├─ SubIssuePlan
├─ DependencyGraph
├─ VerificationPlan
├─ PlanValidation
├─ LinearDraft
├─ ReviewComments
└─ PublishReceipt
```

The planning service should extend the existing `create-implementation-plan` and `convert-tasks-to-linear` skills by wrapping them in a persistent artifact and review workflow. GSD-2 should inform the guided interview, research, analysis, decomposition, and dependency-graph stages.

### 4.7 Event journal and stream broker

The event journal is the durable event source for clients.

Requirements:

- Monotonic sequence numbers.
- Stable event IDs.
- Entity references for project, milestone, issue, sub-issue, run, workspace, terminal, file, approval, and planning session.
- Event type and schema version.
- Correlation IDs for user actions.
- Timestamp.
- Actor.
- Summary.
- Payload.
- Raw harness payload reference when applicable.

The stream broker provides live delivery.

Requirements:

- Cursor-based replay.
- Backpressure.
- Bounded queues.
- Coalescing for high-frequency view-model events.
- Separate streams for high-volume terminal/log frames if needed.
- Stream health metrics.

### 4.8 Terminal and log service

The terminal/log service converts raw execution output into client-renderable streams.

Responsibilities:

- Associate terminal/log streams with run, workspace, command, issue, and sub-issue.
- Provide scrollback reads.
- Provide live frames.
- Support search.
- Support jump-to-event.
- Support terminal snapshots and cell deltas where a parsed terminal model is available.
- Preserve raw output for diagnostics.

Recommended stream model:

- Control events: JSON event envelopes.
- Terminal/log frames: binary frames for high-volume streams, with a versioned schema decodable by Rust and TypeScript.
- Snapshot reads: REST detail endpoints for scrollback and current viewport state.

For desktop local mode, native Rust channels or local IPC can carry high-throughput frames into the Tauri backend, and Tauri channels can carry decoded or binary frame payloads from Rust to the webview. Use zero-copy-friendly Rust representations such as shared byte buffers internally where practical, but keep the public frame schema versioned and decodable by TypeScript. For web and hosted mode, WebSocket binary frames should carry equivalent payloads.

Remote transport should preserve consistency over raw throughput. The recommended baseline is REST/HTTP for snapshots, detail reads, and idempotent mutations, plus WSS streams for ordered events and high-volume frames. JSON-RPC 2.0 over WebSocket is a viable candidate for bidirectional hosted control if benchmarks show it improves correlation, retries, and subscription management. If selected, JSON-RPC must still use OpenSymphony event cursors, idempotency keys, action receipts, monotonic sequence numbers, replay after reconnect, and explicit auth/RBAC checks.

### 4.9 Auth, RBAC, and secrets

Local mode may run trusted and unauthenticated on loopback. Hosted mode requires auth.

Hosted auth responsibilities:

- User identity.
- Organization and tenant membership.
- Role-based permissions.
- Project access.
- Repository access.
- Linear connection access.
- Harness and model access.
- Audit logging.

Secrets responsibilities:

- Store provider keys, Linear credentials, repository credentials, and harness credentials.
- Scope secrets by user, organization, project, and environment.
- Avoid exposing secrets to frontend payloads.
- Provide redaction in logs and events.
- Support rotation and revocation.

Subscription sign-in must use a separate credential type from API-compatible `LLM_API_KEY` settings and should not be owned by a single harness adapter. OpenAI ChatGPT/Codex is the first subscription credential adapter.

## 5. Client-side architecture

### 5.1 Shared frontend package

Recommended frontend package structure:

```text
apps/
├─ desktop-tauri/
│  ├─ src-tauri/
│  └─ webview entry
├─ web/
│  └─ browser entry
packages/
├─ ui-core/
│  ├─ components
│  ├─ panes
│  ├─ layout
│  └─ design tokens
├─ state/
│  ├─ reducers
│  ├─ stores
│  ├─ selectors
│  └─ stream reconciliation
├─ api-client/
│  ├─ gateway REST client
│  ├─ gateway stream client
│  ├─ schema types
│  └─ transport adapters
├─ terminal-renderer/
│  ├─ worker
│  ├─ canvas renderer
│  ├─ frame decoder
│  └─ scrollback model
├─ planning-ui/
├─ taskgraph-ui/
└─ run-ui/
```

The frontend framework can be React, Solid, or another TypeScript framework. The architecture should avoid binding core state and transport contracts to framework-specific internals.

### 5.2 Transport adapters

```ts
interface GatewayTransport {
  getSnapshot(scope: SnapshotScope): Promise<Snapshot>;
  getDetail<T>(request: DetailRequest): Promise<T>;
  mutate<T>(request: MutationRequest): Promise<ActionReceipt<T>>;
  openEventStream(request: StreamRequest): EventStream;
  openBinaryStream(request: BinaryStreamRequest): BinaryStream;
  capabilities(): Promise<GatewayCapabilities>;
}
```

Adapters:

- `DesktopLocalNativeTransport`: in-process Rust channels or native IPC to a local OpenSymphony host, plus Tauri channels to the webview for high-volume streams.
- `DesktopLocalGatewayTransport`: loopback HTTP/WebSocket to a local gateway for compatibility, debugging, and contract tests.
- `DesktopRemoteTransport`: HTTP/WebSocket from the Tauri webview to a remote gateway.
- `BrowserTransport`: HTTP/WebSocket in the browser.
- `TestTransport`: deterministic fixtures for UI and reducer tests.

The frontend should select a transport by connection profile and gateway capabilities. All adapters must reduce to the same client state transitions so the desktop app can switch between local and hosted profiles without changing the UI model.

### 5.3 Client state model

The client state model should be event-reduced.

Flow:

1. Fetch initial snapshot.
2. Open event stream with snapshot sequence.
3. Apply events through reducers.
4. Fetch detail records on demand.
5. Coalesce render-heavy updates.
6. Persist layout and selected entity locally.
7. On reconnect, refetch snapshot or replay from last cursor.
8. Mark stale state clearly during degraded connections.

State should include:

- Connection status.
- Capability state.
- Entity cache.
- Task graph view model.
- Run view model.
- Terminal/log stream state.
- Planning session state.
- Approval state.
- Layout state.

### 5.4 Main UI surfaces

#### Dashboard

- Gateway health.
- Harness health.
- Linear sync health.
- Active runs.
- Queue depth.
- Blocked work.
- Retry queue.
- Recent events.
- Cost/token summary where available.

#### Project and task graph

- Project list.
- Milestone tree.
- Issue and sub-issue list.
- Dependency graph.
- Runtime overlays.
- Filters by status, owner, harness, state, and failure reason.

#### Run detail

- Summary.
- Issue/sub-issue context.
- Workspace metadata.
- Harness session metadata.
- Event timeline.
- Terminal/log panes.
- Diff viewer.
- Validation results.
- Approval requests.
- Action bar.

#### Planning workspace

- Conversation pane.
- Structured artifact editor.
- Milestone/issue/sub-issue hierarchy editor.
- Requirements and risk panes.
- Repository analysis pane.
- Verification plan editor.
- Linear draft preview.
- Publish receipt.

#### Approval center

- Pending approvals.
- Risk summary.
- Related run, command, file, issue, and workspace.
- Approve, deny, explain, or defer.
- Audit history.

## 6. Data model

### 6.1 Core entities

```text
Organization
User
Project
Repository
TrackerConnection
LinearProjectRef
Milestone
Issue
SubIssue
Run
Workspace
HarnessSession
TerminalSession
Event
ApprovalRequest
PlanningSession
Artifact
SecretReference
ModelConfigurationProfile
HarnessProfile
```

### 6.2 Entity relationships

```text
Organization 1..n Users
Organization 1..n Projects
Project 1..n Repositories
Project 1..n Milestones
Milestone 1..n Issues
Issue 1..n SubIssues
SubIssue 0..n Runs
Run 1 Workspace
Run 1 HarnessSession
Run 0..n TerminalSessions
Run 0..n Events
Run 0..n ApprovalRequests
Project 0..n PlanningSessions
PlanningSession 0..n Artifacts
```

### 6.3 Event envelope

```json
{
  "id": "evt_...",
  "seq": 123456,
  "schema_version": "1.0",
  "type": "run.event.normalized",
  "timestamp": "2026-05-10T12:00:00Z",
  "actor": {
    "kind": "system|user|agent|harness",
    "id": "..."
  },
  "correlation_id": "act_...",
  "entity_refs": {
    "project_id": "...",
    "milestone_id": "...",
    "issue_id": "...",
    "sub_issue_id": "...",
    "run_id": "...",
    "workspace_id": "..."
  },
  "summary": "Agent run started",
  "payload": {},
  "raw_ref": "raw_evt_..."
}
```

## 7. API design

### 7.1 Read endpoints

```text
GET /healthz
GET /readyz
GET /api/v1/capabilities
GET /api/v1/dashboard/snapshot
GET /api/v1/projects
GET /api/v1/projects/{project_id}
GET /api/v1/projects/{project_id}/taskgraph
GET /api/v1/milestones/{milestone_id}
GET /api/v1/issues/{issue_id}
GET /api/v1/sub-issues/{sub_issue_id}
GET /api/v1/runs/{run_id}
GET /api/v1/runs/{run_id}/events
GET /api/v1/runs/{run_id}/files
GET /api/v1/runs/{run_id}/diffs
GET /api/v1/runs/{run_id}/logs
GET /api/v1/runs/{run_id}/terminal/{terminal_id}/snapshot
GET /api/v1/planning/sessions/{session_id}
GET /api/v1/planning/sessions/{session_id}/artifacts
```

### 7.2 Stream endpoints

```text
GET  /api/v1/events?cursor={seq}
GET  /api/v1/projects/{project_id}/events?cursor={seq}
GET  /api/v1/runs/{run_id}/events?cursor={seq}
WS   /api/v1/streams/events
WS   /api/v1/streams/runs/{run_id}
WS   /api/v1/streams/terminal/{terminal_id}
WS   /api/v1/streams/planning/{session_id}
```

SSE can remain useful for simple snapshot streams. WebSocket should be preferred for richer hosted and bidirectional client flows. Binary frames should be used for high-volume terminal/log payloads when benchmarks justify them.

If the hosted gateway adopts JSON-RPC 2.0 over WebSocket, use it as a session/control envelope over these same stream concepts. Requests and notifications must carry correlation IDs, auth context, cursor positions where applicable, and resumable subscription identifiers.

### 7.3 Mutation endpoints

```text
POST /api/v1/actions/dispatch
POST /api/v1/actions/retry
POST /api/v1/actions/cancel
POST /api/v1/actions/pause
POST /api/v1/actions/resume
POST /api/v1/actions/rehydrate
POST /api/v1/actions/comment
POST /api/v1/actions/transition-issue
POST /api/v1/actions/create-followup
POST /api/v1/actions/approval-decision
POST /api/v1/taskgraph/milestones
POST /api/v1/taskgraph/issues
POST /api/v1/taskgraph/sub-issues
POST /api/v1/planning/sessions
POST /api/v1/planning/sessions/{session_id}/message
POST /api/v1/planning/sessions/{session_id}/generate
POST /api/v1/planning/sessions/{session_id}/publish-linear
```

Mutation responses should include:

- Action ID.
- Correlation ID.
- Accepted/rejected status.
- Reason.
- Expected events.
- Result payload when immediate.

## 8. OpenHands model configuration, subscription auth, and future Codex architecture

### 8.1 Separation of concerns

The model configuration, subscription credential, and future Codex integration must separate:

- API-compatible OpenHands model configuration through `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`.
- Subscription credential adapters that can construct subscription-backed OpenHands `LLM` objects.
- OpenAI ChatGPT/Codex as the first subscription credential adapter.
- Existing OpenHands agent-server execution using that `LLM`.
- Codex app-server harness execution.
- Model configuration metadata for UI and routing.
- User or organization credential ownership.

### 8.2 Model and credential settings

```text
ModelSettings
├─ mode: api_key | subscription
├─ owner: user | organization | project
├─ base_url
├─ model
├─ api_key_ref
├─ subscription_credential_ref
├─ subscription_provider
├─ credential_storage: local_keychain | openhands_auth_directory | hosted_secret_store
└─ harnesses
```

API key mode maps directly to the existing OpenHands configuration contract. Users configure the base URL, model string, and API key reference; OpenSymphony stores and displays those configured settings. Subscription mode uses provider-specific credential adapters behind a common settings shape. The first adapter is OpenAI ChatGPT/Codex subscription login through documented OpenHands SDK or OpenAI/Codex client flows. Desktop can use browser or device-code login where supported. Hosted mode needs a careful per-user or organization-scoped credential model, should keep refresh tokens in a credential broker or encrypted secret store, and must avoid exposing raw access tokens to browsers. For OpenHands agent-server, the output of subscription login is still an `LLM` attached to an OpenHands `Agent` and `Conversation`; the harness adapter receives model settings and credential references for the run.

### 8.3 Model configuration metadata

```text
ModelConfigurationProfile
├─ id
├─ base_url
├─ model
├─ credential_ref
├─ harnesses: openhands_agent_server | codex_app_server | other
├─ context_window
├─ reasoning_effort
├─ cost_profile
└─ recommended_for
```

The configuration profile records settings and optional operator-supplied metadata for dynamic routing. API-compatible model profiles use the configured base URL, model string, credential reference, and harness capability.

### 8.4 Codex app-server harness shape

```text
CodexHarnessAdapter
├─ transport: stdio | websocket
├─ auth: model_settings_ref | inherited_subscription_login | capability_token | signed_bearer
├─ schema: generated TypeScript and JSON Schema per Codex version
├─ entities: thread, turn, approval, tool call, message, event
├─ streams: JSON-RPC notifications
├─ actions: start thread, start turn, send input, approve, cancel, resume
└─ normalization: Codex event to OpenSymphony event envelope
```

Initial implementation should prepare the interfaces and settings. Production enablement should come after a benchmark and contract-test phase. Codex app-server must reuse model and credential settings rather than becoming the owner of subscription credentials.

## 9. Hosted mode architecture

### 9.1 Hosted components

```text
Hosted OpenSymphony
├─ Gateway/API service
├─ Web frontend service, optional
├─ Auth service or auth middleware
├─ Orchestrator workers
├─ Task graph sync workers
├─ Planning workers
├─ Harness runtime manager
├─ OpenHands agent-server fleet
├─ Future Codex app-server runtime pool
├─ Workspace sandbox layer
├─ Postgres or equivalent relational store
├─ Event journal storage
├─ Artifact/log object storage
├─ Secret store
├─ Metrics/logging/audit stack
└─ Admin interface
```

### 9.2 Hosted execution lifecycle

1. User connects through web or desktop.
2. User selects organization, project, and repository.
3. Gateway verifies permissions.
4. Orchestrator schedules eligible work.
5. Workspace manager creates a hosted workspace.
6. Harness manager starts or resumes an agent session.
7. Runtime events are normalized into the event journal.
8. Clients subscribe to replayable streams.
9. Clients can disconnect.
10. Server continues execution.
11. Clients reconnect and resume from event cursors.
12. Completion evidence updates task graph and Linear.

### 9.3 Hosted security

Hosted mode requires:

- TLS.
- Authenticated API and stream access.
- Tenant isolation in data, storage, logs, events, and workspaces.
- Server-side permission checks for every mutation.
- Workspace isolation through containers, VMs, or managed sandboxes.
- Secret redaction and scoped secret injection.
- Audit logs for sensitive actions.
- Resource quotas and kill controls.
- Admin-configurable retention policies.

## 10. Technology recommendations

### 10.1 Rust backend

- Keep orchestration, gateway, workspace, harness adapters, task graph, event journal, and planning services in Rust.
- Use async Rust consistently across gateway and runtime streams.
- Use typed domain models and versioned schemas.
- Use integration tests with fake harnesses and fake Linear GraphQL responses.

### 10.2 Frontend

- Use TypeScript.
- Use a component framework that supports shared desktop and web builds.
- Use reducer-driven state management for event streams.
- Use worker-based terminal rendering.
- Keep transport adapters separate from UI components.
- Generate schema types from Rust/OpenAPI/JSON Schema where possible.

### 10.3 Streaming schemas

- Use JSON envelopes for control events.
- Use binary frames for terminal/log payloads when throughput requires it.
- Keep binary schemas versioned and testable in Rust and TypeScript.
- Include sequence numbers and stream IDs in every high-volume frame.

### 10.4 Storage

Local mode:

- Local state DB for gateway state and event journal.
- Filesystem workspace storage.
- OS keychain for desktop secrets.

Hosted mode:

- Relational DB for organizations, users, projects, task graph cache, runs, events, and planning sessions.
- Object storage for logs, artifacts, raw harness payloads, and large diff bundles.
- Secret manager for credentials.

## 11. Risk register

### 11.1 Harness protocol drift

OpenHands and Codex app-server protocols can change. Mitigation: version pinning, generated schemas, contract tests, unknown-event preservation, and capability discovery.

### 11.2 Codex WebSocket maturity

Codex app-server WebSocket transport is experimental. Mitigation: prefer stdio for local Codex integration, treat WebSocket as feature-gated, and require auth plus load testing before remote exposure.

### 11.3 Subscription credential constraints

Subscription credentials can be consumed by OpenHands agent-server before Codex app-server exists. Mitigation: keep subscription credentials in dedicated credential settings, use documented login flows, keep refresh tokens out of workspaces, and route runs from configured model settings and harness capability.

### 11.4 Hosted workspace isolation

Hosted execution can run arbitrary code. Mitigation: container/VM isolation, strict network and filesystem policy, secrets scoping, audit logs, quotas, and controlled cleanup.

### 11.5 Event volume and UI performance

Long-running agent sessions can generate large outputs. Mitigation: separate event streams from terminal/log streams, use binary frames where needed, coalesce rendering, maintain durable history server-side, and use worker rendering.

### 11.6 Linear schema drift

Linear GraphQL schema and project milestone behavior can change. Mitigation: schema introspection checks, contract tests, query asset versioning, and guarded mutations.

### 11.7 Planning quality

Generated project plans can be over-broad, under-specified, or poorly decomposed. Mitigation: human review gates, acceptance criteria checks, dependency checks, verification plan requirements, and diffable plan revisions.

## 12. Migration path from current implementation

1. Preserve existing OpenSymphony CLI and local orchestrator behavior.
2. Expand the current read-only control plane into the versioned gateway.
3. Keep FrankenTUI as an optional operator surface.
4. Add rich clients as gateway clients, not replacements for orchestrator logic.
5. Move task graph joins into a gateway service while preserving existing GraphQL helper assets.
6. Add mutation APIs only after read APIs and event streams are stable.
7. Introduce hosted auth and tenancy only after local/external gateway contracts are stable.
8. Add subscription credential support for OpenHands agent-server, starting with OpenAI ChatGPT/Codex, and add Codex app-server later through the harness abstraction after both seams have been tested with local and hosted flows.

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
- COE-396: Action Receipts And Initial Run Actions
- COE-397: Gateway API Client, Transport Adapters, And Reducers
- COE-398: Tauri Shell And Security Capabilities
- COE-402: App Shell, Dashboard, Task Graph, And Run Views
- COE-403: Terminal And Log Renderer Prototype
- COE-404: Desktop Connection Profiles And Daemon Management
- COE-406: Repository, Linear, And Research Analysis
- COE-409: Desktop Settings, Keychain, And Native Actions
- COE-410: Desktop Local Stream Optimization
- COE-413: Implementation Plan Generator Stage

## Source refs

- COE-389
- COE-390
- COE-391
- COE-392
- COE-393
- COE-394
- COE-396
- COE-397
- COE-398
- COE-402
- COE-403
- COE-404
- COE-406
- COE-409
- COE-410
- COE-413

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
