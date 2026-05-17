# Gateway DTO Boundary Checklist

> Identifies private orchestrator structs and fields that must not leak into public client contracts, plus the DTO boundaries needed for gateway work. Supports `P0.1` → `P1.1` migration in `docs/host-client-implementation_plan.md`.

## Risk levels

- 🔴 **High**: Private struct/field currently exposed or at high risk of exposure. Must have DTO boundary before any client work.
- 🟡 **Medium**: Private struct/field used internally but could be derived for gateway. Needs clear mapping decision.
- 🟢 **Low**: Already a public DTO or clearly internal with no leakage path.

## 1. Orchestrator Scheduler (`crates/opensymphony-orchestrator`)

| Item | Risk | Boundary Required | Owner | Notes |
|:-----|:----:|:------------------|:------|:------|
| `Scheduler<T, W, M>` | 🔴 | Never expose directly. Gateway uses `OrchestratorSnapshot` only. | `COE-390` (gateway schemas) | Contains generic tracker/workspace/worker backends, mutable state, and `BTreeMap<IssueId, IssueExecution>`. |
| `Scheduler::executions()` | 🔴 | Remove from public API or return read-only DTOs. | `COE-390` | Currently returns `&BTreeMap<IssueId, IssueExecution>`, giving direct access to mutable orchestrator state machine. |
| `Scheduler::execution()` | 🔴 | Return gateway `ExecutionSummary` DTO instead of `&IssueExecution`. | `COE-390` | Same risk — exposes internal state machine reference. |
| `Scheduler::tracker()` / `tracker_mut()` / `workspace()` / `workspace_mut()` / `worker()` / `worker_mut()` | 🔴 | These are backend access methods. Gateway should never expose backend internals. Keep private or move to internal-only trait. | `COE-390` | Needed by CLI/bootstrap code today. Consider `pub(crate)` visibility. |
| `Scheduler::config()` | 🟡 | `SchedulerConfig` is mostly safe, but `max_concurrent_agents_by_state` is scheduling detail. Gateway can expose a capability subset. | `COE-390` | Map to gateway capability DTO. |
| `Scheduler::bootstrap()` | 🟢 | Internal only. No gateway exposure. | — | |
| `Scheduler::tick()` | 🟢 | Internal only. No gateway exposure. | — | |
| `Scheduler::run_until_shutdown()` | 🟢 | Internal only. No gateway exposure. | — | |
| `Scheduler::snapshot()` | 🟢 | Already safe — returns `OrchestratorSnapshot` DTO. | — | |

## 2. IssueExecution State Machine (`crates/opensymphony-domain::state_machine`)

| Item | Risk | Boundary Required | Owner | Notes |
|:-----|:----:|:------------------|:------|:------|
| `IssueExecution` | 🔴 | Never expose directly. Gateway uses `IssueSnapshot` or future `RunSummary` DTOs. | `COE-390` | Mutable state machine with transitions (`claim`, `start_running`, `queue_retry`, `release`, `reopen`). Exposing this to clients breaks orchestrator-owned scheduling invariant. |
| `IssueExecution::state()` | 🟡 | Currently needed by CLI/snapshot mapping. Return `SchedulerStatus` enum only, not the full tagged enum. | `COE-390` | `SchedulerState` has internal details (`run`, `stall`, `retry`, `released_at`) that belong in `IssueSnapshot`, not in a live ref. |
| `IssueExecution::conversation()` | 🟡 | Returns `&ConversationMetadata`. Safe as read-only DTO, but mutable `update_conversation()` must remain private. | `COE-390` | Gateway should clone, not reference. |
| `IssueExecution::workspace()` | 🟡 | Returns `&WorkspaceRecord`. Safe read-only, but `attach_workspace()` must stay private. | `COE-390` | Gateway clones for DTO. |
| `IssueExecution::retry()` | 🟡 | Returns `&RetryEntry`. Safe read-only. | `COE-390` | |
| `IssueExecution::current_run()` | 🟡 | Returns `&RunAttempt`. Safe read-only. | `COE-390` | |
| `IssueExecution::last_worker_outcome()` / `recent_worker_outcomes()` | 🟢 | Read-only accessors. Already safe. | — | |
| `IssueExecution::snapshot()` | 🟢 | Already safe — returns `IssueSnapshot` DTO. | — | |
| `SchedulerState` (tagged enum) | 🔴 | Internal state representation. Gateway DTO should use `RuntimeStateSnapshot` instead. | `COE-390` | `SchedulerState::Running { run, stall }` contains live `RunAttempt` and `StallMetadata` that must not escape. |
| `TransitionAction` | 🟢 | Internal only. Audit logging may expose action names, but the enum itself stays private. | — | |
| `StateTransitionError` | 🟡 | Error type used by internal transitions. Gateway may need a sanitized error DTO for action rejections. | `COE-390` | Do not expose internal path mismatch details to clients. |

## 3. Runtime types (`crates/opensymphony-domain::runtime`)

| Item | Risk | Boundary Required | Owner | Notes |
|:-----|:----:|:------------------|:------|:------|
| `RunAttempt` | 🟡 | Struct is mostly data, but `record_turn_started()` is a mutable operation. Gateway DTO should be a clone without methods. | `COE-390` | `RunAttempt` has `mark_started()` and `record_turn_started()`. Gateway should not receive mutable references. |
| `RetryEntry` | 🟢 | Data-only struct. Safe to expose as DTO. | — | Factory methods (`continuation()`, `failure()`) stay internal. |
| `ConversationMetadata` | 🟡 | Data-only struct. `observe_event()` and `add_tokens()` are mutable. Gateway should clone and not expose methods. | `COE-390` | This is the primary `HarnessSession` DTO. Will be a key gateway type. |
| `WorkspaceRecord` | 🟢 | Data-only struct. Safe as DTO. | — | |
| `StallMetadata` | 🟢 | Data-only struct. Safe as DTO. | — | |
| `WorkerOutcomeRecord` | 🟢 | Data-only struct. Safe as DTO. | — | |
| `RetryPolicy` | 🟢 | Data-only struct. Safe as DTO. | — | |

## 4. Snapshot types (`crates/opensymphony-domain::snapshot`)

| Item | Risk | Boundary Required | Owner | Notes |
|:-----|:----:|:------------------|:------|:------|
| `OrchestratorSnapshot` | 🟢 | Already a clean DTO. Safe for gateway. | — | Gateway should add `api_version` field. |
| `IssueSnapshot` | 🟢 | Already a clean DTO. Safe for gateway. | — | Contains `NormalizedIssue` + `RuntimeStateSnapshot` + `WorkspaceRecord` + `ConversationMetadata` + `RetrySnapshot` + outcomes. All sub-types are safe. |
| `DaemonSnapshot` (internal) | 🟡 | Used by `OrchestratorSnapshot`. Safe within the snapshot, but the separate `ControlPlaneDaemonSnapshot` (control-plane version) is what clients see. | `COE-390` | Consider merging or aliasing these two `DaemonSnapshot` types to avoid confusion. |
| `RuntimeStateSnapshot` | 🟢 | Clean read-only DTO. Safe for gateway. | — | |
| `WorkerAttemptSnapshot` | 🟢 | Clean read-only DTO. Safe for gateway. | — | |
| `RetrySnapshot` | 🟢 | Clean read-only DTO. Safe for gateway. | — | |
| `ComponentHealthSnapshot` | 🟢 | Clean read-only DTO. Safe for gateway. | — | |
| `RuntimeUsageTotals` | 🟢 | Clean read-only DTO. Safe for gateway. | — | |

## 5. Control Plane types (`crates/opensymphony-domain::control_plane`)

| Item | Risk | Boundary Required | Owner | Notes |
|:-----|:----:|:------------------|:------|:------|
| `ControlPlaneDaemonSnapshot` | 🟢 | This is the current client-facing DTO. Safe for gateway v1. | — | Will evolve into `DashboardSnapshot` in gateway v1.1. |
| `SnapshotEnvelope` | 🟢 | Already a clean wrapper with sequence + timestamp. Safe for gateway. | — | Gateway should preserve this envelope pattern for all snapshot types. |
| `ControlPlaneIssueSnapshot` | 🟢 | Client-facing DTO. Safe for gateway. | — | Note: `conversation_id_suffix` and `workspace_path_suffix` are display fields. Gateway v1.2 may replace with full `ConversationMetadata` and `WorkspaceRecord`. |
| `ControlPlaneMetricsSnapshot` | 🟢 | Client-facing DTO. Safe for gateway. | — | |
| `ControlPlaneAgentServerStatus` | 🟢 | Client-facing DTO. Safe for gateway. | — | |
| `ControlPlaneDaemonStatus` | 🟢 | Client-facing DTO. Safe for gateway. | — | |
| `ControlPlaneRecentEvent` | 🟢 | Client-facing DTO. Safe for gateway. | — | |
| `ControlPlaneConversationEvent` | 🟢 | Client-facing DTO. Safe for gateway. | — | |
| `ControlPlaneFileChange` | 🟢 | Client-facing DTO. Safe for gateway. | — | |

## 6. OpenHands Runtime (`crates/opensymphony-openhands`)

| Item | Risk | Boundary Required | Owner | Notes |
|:-----|:----:|:------------------|:------|:------|
| `OpenHandsClient` | 🔴 | Never expose to gateway clients. Gateway should abstract harness interactions behind a `HarnessAdapter` trait. | `COE-390` + future harness work | Client has auth credentials, HTTP internals, and stream sockets. |
| `LocalServerSupervisor` | 🔴 | Never expose to gateway clients. Server lifecycle is orchestrator-private. | `COE-390` (gateway schemas / harness abstraction) | |
| `ConversationStore` | 🔴 | Never expose. File system paths and conversation storage are orchestrator-private. | `COE-390` (gateway schemas / harness abstraction) | |
| `EventEnvelope` | 🟡 | Raw harness event payload (`serde_json::Value`). Gateway should normalize to a stable event DTO while preserving raw payload references. | `COE-390` | Current `KnownEvent` enum is a good start but is harness-specific. Future gateway needs a harness-agnostic event envelope. |
| `KnownEvent` | 🟡 | Harness-specific event parsing. Gateway should use a generic event DTO with `kind`, `payload`, and optional `harness_kind` field. | `COE-390` | |
| `IssueSessionRunner` | 🔴 | Never expose. This is the orchestrator's worker backend implementation. | `COE-390` | |
| `IssueSessionContext` / `Manifest` / `LaunchProfile` | 🔴 | Orchestrator-private context for launching runs. | `COE-390` | |
| `RehydrationOptions` / `RehydrationResult` | 🟡 | May become a gateway action input (`RehydrateRun` action), but the internal machinery stays private. | `COE-390` | |
| `AgentConfig` / `LlmConfig` | 🟡 | Contains `api_key`. Must be redacted in any gateway DTO. | `COE-390` + auth work | Gateway should expose model name, base URL (sanitized), and credential reference — never the raw key. |
| `TransportConfig` / `AuthConfig` | 🔴 | Contains auth secrets. Never expose. | `COE-390` + auth work | |

## 7. Linear Integration (`crates/opensymphony-linear`)

| Item | Risk | Boundary Required | Owner | Notes |
|:-----|:----:|:------------------|:------|:------|
| `LinearClient` | 🔴 | Never expose to gateway clients. Linear API key is inside. | `COE-390` + `COE-391` | Gateway may expose `TrackerAdapter` trait with capability discovery, but never the concrete client. |
| `LinearConfig` | 🟡 | Contains `api_key`. Must be redacted in gateway DTOs. | `COE-390` + auth work | |
| `GraphqlError` / `LinearError` | 🟡 | Error details may contain sensitive query text. Gateway should sanitize to `TrackerErrorCategory` + message. | `COE-390` | |

## 8. CLI (`crates/opensymphony-cli`)

| Item | Risk | Boundary Required | Owner | Notes |
|:-----|:----:|:------------------|:------|:------|
| `ResolvedWorkflow` | 🔴 | Contains `LINEAR_API_KEY` and other secrets. Never expose. | `COE-390` + auth work | |
| `orchestrator_run::backends` | 🔴 | Backend instantiation is CLI-private. Gateway uses pre-configured adapters. | `COE-390` | |
| `map_snapshot()` / `map_issue()` | 🟢 | Safe mapping functions. These are the DTO boundary *implementation*. | — | These functions model what the gateway should do: transform `OrchestratorSnapshot` → `ControlPlaneDaemonSnapshot`. |

## 9. Summary: boundary rules for gateway v1

1. **No mutable orchestrator state escapes**: `IssueExecution`, `SchedulerState`, `Scheduler` must never be referenced by the gateway. Only `IssueSnapshot`, `OrchestratorSnapshot`, and derived DTOs may cross the boundary.
2. **No backend internals escape**: `OpenHandsClient`, `LinearClient`, `LocalServerSupervisor`, backend traits, and transport configs stay orchestrator-private.
3. **No secrets escape**: `api_key`, `LINEAR_API_KEY`, auth tokens, and credential configs must be redacted or replaced with capability/credential-reference DTOs.
4. **All client-facing types get `api_version`**: Every snapshot, detail, and event envelope must carry a version field for forward compatibility.
5. **Raw harness payloads get normalized**: `EventEnvelope` (raw JSON) should be converted to a stable gateway event DTO, with raw payload stored server-side and referenced by ID.
6. **Action inputs are DTOs, not internal structs**: Gateway actions (retry, cancel, rehydrate, comment) must define their own request DTOs and translate to orchestrator operations internally.

## 10. Follow-up task mapping

| Follow-up | Blocked by | Scope | Acceptance |
|:----------|:-----------|:------|:-----------|
| `COE-390` Gateway schemas and stream feasibility | `COE-389` | JSON schemas / Rust DTOs for gateway v1; capability discovery; action receipt framework | Draft schemas + benchmark evidence |
| `COE-391` Gateway module capabilities and dashboard snapshot | `COE-389`, `COE-390` | Implement `/api/v1/capabilities`, `/api/v1/dashboard/snapshot`, module boundary | Tests + fixture validation |
| `COE-392` Task graph run detail file and diff read APIs | `COE-389`, `COE-390` | Run detail, event history with cursor, changed files, per-file diffs | API tests + file safety tests |
| Future: harness abstraction v2 | `COE-390` | Generic `HarnessAdapter` trait; `CodexHarnessAdapter` spike | Interface review + fixture tests |
| Future: hosted auth and identity | `COE-390`, `COE-391` | Credential storage, redaction, refresh tokens, tenant isolation | Threat model + integration tests |
