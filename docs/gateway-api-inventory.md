# Current Gateway API Inventory

> Inventory of the existing OpenSymphony control plane, orchestrator surfaces, and runtime contracts. This document captures repo paths and representative payload shapes as of the current codebase. It is the deliverable for `P0.1` in `docs/host-client-implementation_plan.md`.
>
> **Note:** File paths and line numbers cited below are approximate and refer to the state of the repository as of this commit. They will drift over time; use the paths as module-level navigation hints rather than exact references.

## 1. Control Plane Server (`crates/opensymphony-control`)

### 1.1 Endpoints

| Method | Path | Handler | Response Type | File |
|:------:|:-----|:--------|:--------------|:-----|
| `GET`  | `/healthz` | `health` | `HealthResponse` | `crates/opensymphony-control/src/lib.rs:121` |
| `GET`  | `/api/v1/snapshot` | `snapshot` | `SnapshotEnvelope` | `crates/opensymphony-control/src/lib.rs:131` |
| `GET`  | `/api/v1/events` | `events` | SSE stream (`Event`) | `crates/opensymphony-control/src/lib.rs:135` |

### 1.2 `HealthResponse` shape

```json
{
  "status": "ok",
  "current_sequence": 42,
  "published_at": "2026-03-21T20:00:00Z",
  "issue_count": 3
}
```

### 1.3 `SnapshotEnvelope` shape

```json
{
  "sequence": 42,
  "published_at": "2026-03-21T20:00:00Z",
  "snapshot": { /* ControlPlaneDaemonSnapshot */ }
}
```

### 1.4 `ControlPlaneDaemonSnapshot` shape

```json
{
  "generated_at": "2026-03-21T20:00:00Z",
  "daemon": {
    "state": "ready",
    "last_poll_at": "2026-03-21T20:00:00Z",
    "workspace_root": "/tmp/opensymphony",
    "status_line": "poll=1000ms, running=1, retry_queue=0"
  },
  "agent_server": {
    "reachable": true,
    "base_url": "http://127.0.0.1:3000",
    "conversation_count": 2,
    "status_line": "healthy"
  },
  "metrics": {
    "running_issues": 1,
    "retry_queue_depth": 0,
    "input_tokens": 2048,
    "output_tokens": 2048,
    "cache_read_tokens": 512,
    "total_tokens": 4096,
    "total_cost_micros": 120000
  },
  "issues": [ /* ControlPlaneIssueSnapshot[] */ ],
  "recent_events": [ /* ControlPlaneRecentEvent[] */ ]
}
```

### 1.5 `ControlPlaneIssueSnapshot` shape

```json
{
  "identifier": "COE-255",
  "title": "Observability and FrankenTUI",
  "tracker_state": "In Progress",
  "runtime_state": "running",
  "last_outcome": "running",
  "last_event_at": "2026-03-21T20:00:00Z",
  "conversation_id_suffix": "c0e255",
  "workspace_path_suffix": "COE-255",
  "retry_count": 0,
  "blocked": false,
  "server_base_url": "http://127.0.0.1:3000",
  "transport_target": "loopback",
  "http_auth_mode": "none",
  "websocket_auth_mode": "none",
  "websocket_query_param_name": null,
  // "recent_events"   — omitted when empty (#[serde(skip_serializing_if = "Vec::is_empty")])
  // "modified_files"  — omitted when empty (#[serde(skip_serializing_if = "Vec::is_empty")])
  "input_tokens": 1024,
  "output_tokens": 512,
  "cache_read_tokens": 256
}
```

### 1.6 `ControlPlaneRecentEvent` shape

```json
{
  "happened_at": "2026-03-21T20:00:00Z",
  "issue_identifier": "COE-255",
  "kind": "snapshot_published",
  "summary": "published step 1"
}
```

### 1.7 SSE Stream behavior

- Server-sent events via `async_stream::stream!` + `axum::response::sse::Sse`.
- Event type: `snapshot`.
- Event `id`: sequence number as string.
- Keepalive interval: 15 seconds (`: keepalive\n\n`).
- Stream attach timeout: 5 seconds.
- Stream read timeout: 35 seconds.
- Lagged receivers fast-forward to latest snapshot instead of draining backlog.
- Deduplication by sequence number on the client.

### 1.8 Control Plane Client

- `ControlPlaneClient::new(base_url)` — `fetch_snapshot()` + `stream_updates()`.
- Path-prefixed base URLs supported (e.g. `http://host/opensymphony`).
- Configurable timeouts via `with_timeouts()`.

## 2. Domain DTOs (`crates/opensymphony-domain`)

### 2.1 Public control-plane types (`src/control_plane.rs`)

| Type | Purpose | File |
|:-----|:--------|:-----|
| `SnapshotEnvelope` | Versioned snapshot wrapper with sequence + timestamp | `src/control_plane.rs:5` |
| `ControlPlaneDaemonSnapshot` | Top-level daemon + metrics + issues + events | `src/control_plane.rs:12` |
| `ControlPlaneDaemonStatus` | Daemon state, last poll, workspace root, status line | `src/control_plane.rs:28` |
| `ControlPlaneDaemonState` | Enum: `starting`, `ready`, `degraded`, `stopped` | `src/control_plane.rs:37` |
| `ControlPlaneAgentServerStatus` | Reachability, base URL, conversation count | `src/control_plane.rs:45` |
| `ControlPlaneMetricsSnapshot` | Token counts, cost micros, queue depth | `src/control_plane.rs:53` |
| `ControlPlaneIssueSnapshot` | Per-issue public runtime view | `src/control_plane.rs:64` |
| `ControlPlaneIssueRuntimeState` | Enum: `idle`, `running`, `retry_queued`, `releasing`, `completed`, `failed` | `src/control_plane.rs:123` |
| `ControlPlaneWorkerOutcome` | Enum: `unknown`, `running`, `continued`, `completed`, `failed`, `canceled` | `src/control_plane.rs:134` |
| `ControlPlaneRecentEvent` | Timestamped event with issue ref and kind | `src/control_plane.rs:143` |
| `ControlPlaneRecentEventKind` | Enum: `worker_started`, `workspace_prepared`, `stream_attached`, `snapshot_published`, `worker_completed`, `retry_scheduled`, `client_attached`, `client_detached`, `warning` | `src/control_plane.rs:153` |
| `ControlPlaneConversationEvent` | Per-conversation activity event | `src/control_plane.rs:98` |
| `ControlPlaneFileChange` | Path, kind, lines added/removed | `src/control_plane.rs:106` |
| `ControlPlaneFileChangeKind` | Enum: `created`, `modified`, `removed` | `src/control_plane.rs:115` |

### 2.2 Identifiers (`src/identifiers.rs`)

| Type | Kind | File |
|:-----|:-----|:-----|
| `ConversationId` | String wrapper | `src/identifiers.rs:59` |
| `IssueId` | String wrapper | `src/identifiers.rs:60` |
| `IssueIdentifier` | String wrapper | `src/identifiers.rs:61` |
| `TrackerStateId` | String wrapper | `src/identifiers.rs:62` |
| `WorkerId` | String wrapper | `src/identifiers.rs:63` |
| `WorkspaceKey` | Sanitized string (`sanitize_workspace_key`) | `src/identifiers.rs:67` |

### 2.3 Runtime types (`src/runtime.rs`)

| Type | Purpose | File |
|:-----|:--------|:-----|
| `WorkspaceRecord` | Path, key, created/updated timestamps | `src/runtime.rs:11` |
| `RetryAttempt` | Non-zero u32 attempt ordinal | `src/runtime.rs:22` |
| `RuntimeStreamState` | Enum: `detached`, `attaching`, `ready`, `reconnecting`, `closed`, `failed` | `src/runtime.rs:74` |
| `ConversationMetadata` | Full harness session metadata + token counts + recent_activity | `src/runtime.rs:83` |
| `ConversationActivityEvent` | Per-activity event (id, timestamp, kind, summary) | `src/runtime.rs:118` |
| `RetryPolicy` | Continuation / failure delays + max backoff | `src/runtime.rs:170` |
| `RetryReason` | Enum: `continuation`, `failure`, `stalled`, `cancelled`, `reconciliation` | `src/runtime.rs:201` |
| `RetryEntry` | Scheduled retry with issue ref, attempt, due time | `src/runtime.rs:210` |
| `RunAttempt` | Worker claim: id, issue, path, claimed/started timestamps, turns | `src/runtime.rs:269` |
| `StallMetadata` | Last activity + stall timeout + stalled_at | `src/runtime.rs:322` |
| `WorkerOutcomeKind` | Enum: `succeeded`, `failed`, `timed_out`, `stalled`, `cancelled` | `src/runtime.rs:348` |
| `WorkerOutcomeRecord` | Outcome with worker, attempt, timestamps, turns, summary, error | `src/runtime.rs:358` |
| `ReleaseReason` | Enum: `completed`, `tracker_inactive`, `tracker_terminal`, `cancelled`, `retry_exhausted` | `src/runtime.rs:392` |

### 2.4 Snapshot types (`src/snapshot.rs`)

| Type | Purpose | File |
|:-----|:--------|:-----|
| `ComponentHealthSnapshot` | Status, detail, updated_at | `src/snapshot.rs:20` |
| `RuntimeUsageTotals` | Aggregated token/cost/runtime metrics | `src/snapshot.rs:27` |
| `DaemonSnapshot` | Internal daemon health + counts + usage | `src/snapshot.rs:37` |
| `WorkerAttemptSnapshot` | Worker id, attempt, retry count, turns, max turns | `src/snapshot.rs:71` |
| `RetrySnapshot` | Retry attempt, count, schedule/due times, reason, error | `src/snapshot.rs:92` |
| `RuntimeStateSnapshot` | Scheduler state + claim/start/release times + worker + event/stall | `src/snapshot.rs:115` |
| `IssueSnapshot` | NormalizedIssue + runtime + workspace + conversation + retry + outcomes | `src/snapshot.rs:188` |
| `OrchestratorSnapshot` | generated_at + daemon + issues (internal orchestrator output) | `src/snapshot.rs:214` |

### 2.5 Issue types (`src/issue.rs`)

| Type | Purpose | File |
|:-----|:--------|:-----|
| `IssueStateCategory` | Enum: `active`, `non_active`, `terminal` | `src/issue.rs:7` |
| `IssueState` | Optional id, name, category | `src/issue.rs:14` |
| `BlockerRef` | Blocker id, identifier, state, timestamps | `src/issue.rs:21` |
| `IssueRef` | Child issue id, identifier, state | `src/issue.rs:30` |
| `NormalizedIssue` | The canonical issue representation used by the orchestrator | `src/issue.rs:37` |

### 2.6 State machine (`src/state_machine.rs`)

| Type | Purpose | File |
|:-----|:--------|:-----|
| `SchedulerStatus` | Enum: `unclaimed`, `claimed`, `running`, `retry_queued`, `released` | `src/state_machine.rs:19` |
| `TransitionAction` | Enum: `claim`, `start_running`, `record_turn_started`, `observe_runtime_event`, `queue_retry`, `release`, `reopen` | `src/state_machine.rs:47` |
| `StateTransitionError` | InvalidTransition, AttemptMismatch, IssueMismatch, WorkspaceNotAttached, WorkspaceIssueMismatch, WorkspaceIdentityMismatch, WorkspacePathMismatch, ConversationNotAttached, WorkerMismatch | `src/state_machine.rs:77` |
| `SchedulerState` | Tagged enum of all scheduler states with details | `src/state_machine.rs:123` |
| `IssueExecution` | Core orchestrator state machine struct; owns issue, workspace, conversation, state, outcomes | `src/state_machine.rs:156` |

### 2.7 Tracker types (`src/tracker.rs`)

| Type | Purpose | File |
|:-----|:--------|:-----|
| `TrackerIssue` | Raw Linear issue with project_milestone, parent, blocked_by, sub_issues | `src/tracker.rs:7` |
| `TrackerIssueStateSnapshot` | Issue id, identifier, state, updated_at | `src/tracker.rs:30` |
| `TrackerIssueState` | id, name, tracker_type, kind | `src/tracker.rs:38` |
| `TrackerIssueBlocker` | Blocker id, identifier, title, state | `src/tracker.rs:53` |
| `TrackerIssueRef` | Issue id, identifier, optional title/url, state | `src/tracker.rs:67` |
| `TrackerProjectMilestone` | Milestone id, name | `src/tracker.rs:87` |
| `TrackerIssueStateKind` | Enum: `backlog`, `unstarted`, `started`, `completed`, `canceled`, `triage`, `unknown(String)` | `src/tracker.rs:93` |
| `TrackerErrorCategory` | Enum: `auth`, `rate_limited`, `transport`, `timeout`, `invalid_response`, `not_found`, `invalid_state_transition`, `permission_denied` | `src/tracker.rs:124` |

## 3. Orchestrator (`crates/opensymphony-orchestrator`)

### 3.1 Public API surface (`src/scheduler.rs`)

| Type | Purpose | File |
|:-----|:--------|:-----|
| `SchedulerConfig` | Poll interval, concurrency, turns, state limits, retry policy, stall timeout, active/terminal states | `src/scheduler.rs:28` |
| `RecoveryRecord` | Issue + workspace + had_in_flight_run flag | `src/scheduler.rs:93` |
| `WorkerStartRequest` | Issue + workspace + run | `src/scheduler.rs:100` |
| `WorkerLaunch` | Conversation metadata returned by harness start | `src/scheduler.rs:107` |
| `WorkerUpdate` | RuntimeEvent / ConversationMetadataUpdate / Finished | `src/scheduler.rs:113` |
| `WorkerAbortReason` | TrackerInactive / TrackerTerminal / Stalled | `src/scheduler.rs:132` |
| `TrackerBackend` | Trait: candidate_issues, terminal_issues, issue_states_by_ids | `src/scheduler.rs:139` |
| `WorkspaceBackend` | Trait: ensure_workspace, recover_workspaces, cleanup_workspace | `src/scheduler.rs:151` |
| `WorkerBackend` | Trait: start_worker, start_workers (default), poll_updates, abort_worker | `src/scheduler.rs:170` |
| `SchedulerError` | InvalidConfiguration, Tracker, Workspace, Worker, StateTransition, RetryCalculation, Identifier | `src/scheduler.rs:198` |
| `Scheduler<T, W, M>` | Core struct with tracker, workspace, worker, config, executions, running_counts_by_state, worker_index, pending_recovery, recovered, next_worker_ordinal, last_poll_at, health | `src/scheduler.rs:216` |

### 3.2 Scheduler methods

| Method | Visibility | Purpose | File |
|:-------|:-----------|:--------|:-----|
| `new` | `pub` | Constructor | `src/scheduler.rs:237` |
| `config` | `pub` | Immutable config ref | `src/scheduler.rs:254` |
| `tracker` | `pub` | Immutable tracker ref | `src/scheduler.rs:258` |
| `tracker_mut` | `pub` | Mutable tracker ref | `src/scheduler.rs:262` |
| `workspace` | `pub` | Immutable workspace ref | `src/scheduler.rs:266` |
| `workspace_mut` | `pub` | Mutable workspace ref | `src/scheduler.rs:270` |
| `worker` | `pub` | Immutable worker ref | `src/scheduler.rs:274` |
| `worker_mut` | `pub` | Mutable worker ref | `src/scheduler.rs:278` |
| `executions` | `pub` | Immutable executions map ref | `src/scheduler.rs:282` |
| `execution` | `pub` | Single execution lookup by IssueId | `src/scheduler.rs:286` |
| `snapshot` | `pub` | Derives `OrchestratorSnapshot` from current state | `src/scheduler.rs:290` |
| `bootstrap` | `pub async` | Recovery pass: loads workspaces, reconciles in-flight runs | `src/scheduler.rs:334` |
| `tick` | `pub async` | Single poll-and-dispatch cycle | `src/scheduler.rs:357` |
| `run_until_shutdown` | `pub async` | Main loop with interval + shutdown signal | `src/scheduler.rs:393` |

### 3.3 Selection logic (`src/selection.rs`)

| Function | Purpose | File |
|:---------|:--------|:-----|
| `issue_blocked_by_non_terminal_blockers` | Checks if any blocker is non-terminal | `src/selection.rs:5` |
| `parent_issue_blocked_by_incomplete_children` | Checks if any child is non-terminal | `src/selection.rs:12` |
| `should_dispatch_issue` | Combines blocker + hierarchy checks | `src/selection.rs:23` |
| `filter_issues_for_dispatch` | Filters + sorts eligible issues | `src/selection.rs:28` |
| `sort_issues_for_dispatch` | Sorts by priority, then child count, then created_at, then identifier | `src/selection.rs:43` |

## 4. Linear Integration (`crates/opensymphony-linear`)

### 4.1 Public API surface (`src/lib.rs`)

| Type | Purpose | File |
|:-----|:--------|:-----|
| `LinearClient` | GraphQL client with retry + auth | `src/lib.rs:6` |
| `LinearConfig` | API key, base URL, project slug, retry policy | `src/lib.rs:6` |
| `RetryPolicy` | Backoff parameters for GraphQL requests | `src/lib.rs:6` |
| `WorkpadComment` | Comment body + optional workpad marker | `src/lib.rs:6` |
| `GraphqlError` | GraphQL error wrapper | `src/lib.rs:7` |
| `LinearError` | Unified error enum | `src/lib.rs:7` |

### 4.2 Internal modules

- `client.rs` — HTTP/GraphQL client implementation, pagination, retry.
- `graphql.rs` — Query/mutation strings and variable builders.
- `normalize.rs` — Normalization from `TrackerIssue` to `NormalizedIssue`.
- `error.rs` — Error types and categorization.

## 5. OpenHands Runtime (`crates/opensymphony-openhands`)

### 5.1 Client surface (`src/client.rs` → `src/lib.rs` re-exports)

| Type | Purpose |
|:-----|:--------|
| `OpenHandsClient` | REST client for conversation create, send, run, search events |
| `OpenHandsError` | Client error enum |
| `OpenHandsProbeResult` | Server readiness probe result |
| `RuntimeEventStream` | WebSocket event stream wrapper |
| `RuntimeStreamConfig` | Stream configuration |
| `TransportConfig` / `TransportAuthKind` / `TransportTargetKind` | Connection routing and auth |
| `TransportDiagnostics` | Debug/diagnostic info |
| `ApiKeyAuth` / `HttpAuth` / `WebSocketAuth` / `AuthConfig` | Auth configurations |

### 5.2 Models (`src/models.rs`)

| Type | Purpose |
|:-----|:--------|
| `ConversationCreateRequest` | POST /conversation payload (workspace, agent, LLM, max_iterations, etc.) |
| `ConversationRunRequest` | POST /conversation/{id}/run payload (empty object) |
| `SendMessageRequest` | POST /conversation/{id}/message payload (role, content[], run flag) |
| `Conversation` | Response shape with id, workspace, agent, stats, execution_status |
| `EventEnvelope` | Runtime event with id, timestamp, source, kind, payload, key, value. Custom Serialize/Deserialize for flattened payload. |
| `SearchConversationEventsResponse` | Paginated event list with next_page_id |
| `AgentConfig` / `LlmConfig` / `CondenserConfig` / `ToolConfig` / `ConfirmationPolicy` / `WorkspaceConfig` | Agent and model configuration |
| `DoctorProbeConfig` | Lightweight config for health-check conversations |
| `TextContent` / `AcceptedResponse` | Message content and generic acceptance |
| `ConversationStateUpdatePayload` | Execution status + state delta |

### 5.3 Events (`src/events.rs`)

| Type | Purpose |
|:-----|:--------|
| `KnownEvent` | Discriminated union: ConversationStateUpdate, LLMCompletionLog, ConversationError, Message, Action, Observation, Unknown |
| `LlmCompletionLogEvent` | Token usage extraction + model name extraction |
| `ConversationErrorEvent` | Error payload wrapper |
| `MessageEventPayload` | Role, content, text preview |
| `ActionEventPayload` | Action id, tool name, message, arguments |
| `ObservationEventPayload` | Observation id, tool name, content, text preview, exit code |
| `UnknownEvent` | Fallback for unrecognized kinds |
| `EventCache` / `ConversationStateMirror` / `ActivityKind` / `ActivitySummary` / `TerminalExecutionStatus` | Event caching and activity summarization |

### 5.4 Session (`src/session.rs`)

| Type | Purpose |
|:-----|:--------|
| `IssueSessionRunner` | Orchestrates a full agent run for an issue: create conversation, send prompt, attach stream, consume events, track state, validate, produce result |
| `IssueSessionContext` / `IssueConversationManifest` / `ConversationLaunchProfile` | Context and manifest types |
| `IssueSessionResult` / `IssueSessionError` | Outcome and error enums |
| `IssueSessionObserver` / `IssueSessionPromptKind` / `IssueSessionReusePolicy` | Observer pattern, prompt kinds, reuse policies |
| `IssueSessionRunnerConfig` / `LlmConfigFingerprint` / `RUNTIME_CONTRACT_VERSION` | Configuration and versioning |
| `RehydrationOptions` / `RehydrationResult` | Conversation reset/rehydration |
| `WorkpadComment` / `WorkpadCommentSource` | Workpad comment types |

### 5.5 Supervisor (`src/supervisor.rs`)

| Type | Purpose |
|:-----|:--------|
| `LocalServerSupervisor` | Manages local OpenHands agent-server process lifecycle |
| `ServerMode` / `ServerState` / `ServerStatus` | Server mode enum, runtime state, status |
| `SupervisorConfig` / `SupervisedServerConfig` / `ExternalServerConfig` / `ProbeConfig` | Configuration structs |
| `SupervisorError` / `LaunchOwnership` | Error enum and ownership tracking |

### 5.6 Tooling (`src/tooling.rs`)

| Type | Purpose |
|:-----|:--------|
| `LocalServerTooling` | Resolves Python environment, openhands-server package, launch command |
| `LocalToolingLayout` / `LocalToolingError` / `PinStatus` / `ResolvedLaunch` / `ToolingMetadata` | Path layout, errors, pin status, resolved launch, metadata |

### 5.7 Conversation Store (`src/conversation_store.rs`)

| Type | Purpose |
|:-----|:--------|
| `OpenHandsConversationStorePaths` | Path resolution for conversation storage |
| `ConversationStoreKind` | Store kind enum |
| `ConversationMoveOutcome` / `ConversationStoreError` / `LocatedConversation` | Move outcomes, errors, located conversations |

## 6. CLI (`crates/opensymphony-cli`)

### 6.1 Commands

| Command | Module | Purpose |
|:--------|:-------|:--------|
| `init` | `init_repo.rs` | Initialize repo with workflow, AGENTS.md, .agents/skills |
| `update` | `update_repo.rs` | Update workflow, skills, AGENTS.md from upstream |
| `install` | `install_tooling.rs` | Install OpenHands server tooling locally |
| `run` | `orchestrator_run/` | Start the orchestrator daemon with scheduler, backends, control plane |
| `tui` | `orchestrator_run/` | Start the FrankenTUI operator interface |
| `debug` | `debug_session.rs` | Launch a debug conversation for an issue |
| `memory` | `memory.rs` / `memory_init_summary.rs` | Initialize or update AGENTS.md memory |

### 6.2 Snapshot mapping (`orchestrator_run/snapshot.rs`)

- `map_snapshot()` converts `OrchestratorSnapshot` → `ControlPlaneDaemonSnapshot`.
- `map_issue()` converts `Domain IssueSnapshot` → `ControlPlaneIssueSnapshot`.
- `map_worker_outcome()` maps `WorkerOutcomeKind` → `ControlPlaneWorkerOutcome`.
- `push_recent_event()` maintains a bounded `VecDeque<RecentEvent>` (limit = 24).
- `terminal_state_set()` derives terminal states from workflow config.

## 7. Event flow summary

```text
Linear API  ──►  LinearClient  ──►  TrackerBackend
                                              │
                                              ▼
                           NormalizedIssue  ──►  IssueExecution (state machine)
                                              │
                                              ▼
                           OrchestratorSnapshot  ──►  map_snapshot()  ──►  ControlPlaneDaemonSnapshot
                                              │                                          │
                                              ▼                                          ▼
                           SnapshotEnvelope  ──►  SnapshotStore.publish()  ──►  SSE /api/v1/events
                                              │                                          │
                                              ▼                                          ▼
                           ControlPlaneClient.fetch_snapshot() / stream_updates()  ──►  TUI / CLI / future Gateway
```

## 8. Test coverage summary

| Crate | Test file | What it tests |
|:------|:----------|:--------------|
| `opensymphony-control` | `tests/control_plane.rs` | Snapshot fetch, SSE stream, path-prefix routing, timeouts, keepalive behavior |
| `opensymphony-domain` | `src/lib.rs` (inline) | State transitions, workspace attachment, retry binding, stall detection, snapshot derivation |
| `opensymphony-orchestrator` | `tests/scheduler.rs` | Scheduler tick, bootstrap, dispatch, retry, cancellation |
| `opensymphony-orchestrator` | `tests/hierarchy_selection.rs` | Parent/child blocking, terminal state filtering, sort order |
| `opensymphony-openhands` | `tests/client_resilience.rs` | Client retry, timeout, error handling |
| `opensymphony-openhands` | `tests/fake_server_contract.rs` | Fake server fixture for create/send/run/search |
| `opensymphony-openhands` | `tests/issue_session_runner.rs` | Full issue session lifecycle |
| `opensymphony-openhands` | `tests/live_local_suite.rs` | Live local server integration |
| `opensymphony-openhands` | `tests/live_pinned_server.rs` | Pinned version contract validation |
| `opensymphony-openhands` | `tests/supervisor.rs` | Server lifecycle supervision |
| `opensymphony-openhands` | `tests/transport_config.rs` | Transport routing and auth |
| `opensymphony-cli` | `tests/debug.rs`, `tests/doctor.rs`, `tests/help.rs`, `tests/init.rs`, `tests/install.rs`, `tests/memory.rs`, `tests/run.rs`, `tests/tui.rs`, `tests/update.rs` | CLI command integration tests |
