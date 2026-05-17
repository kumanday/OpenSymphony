# Domain Vocabulary and ID Mapping

> Defines the public vocabulary (`Project`, `Milestone`, `Issue`, `SubIssue`, `Run`, `Workspace`, `HarnessSession`, `TerminalSession`, `PlanningSession`, `Artifact`) and the Linear-to-OpenSymphony ID mapping rules for local and hosted modes. Consistent with `docs/hosted-client-PRD.md` and `PRODUCT.md`.

## 1. Vocabulary

### 1.1 `Project`

A Linear project combined with a repository execution scope managed by OpenSymphony.

- **Linear mapping**: `Linear Project` (via `project_slug` in workflow config).
- **OpenSymphony representation**: Not yet a first-class tracked entity; currently implied by workflow `project_slug` + repository root.
- **Future gateway entity**: `Project { id, slug, name, repo_root, milestones[], issues[], created_at, updated_at }`.
- **ID rule**: In local mode, the project is identified by `project_slug` (e.g. `e7b957855cb7`). In hosted mode, a stable `ProjectId` (UUID) will be minted by the gateway and mapped to the Linear project.

### 1.2 `Milestone`

A Linear project milestone representing a major delivery stage or checkpoint.

- **Linear mapping**: `Linear ProjectMilestone`.
- **GSD-2 mapping**: GSD-2 milestone / phase-level planning.
- **OpenSymphony representation**: Currently surfaced through `TrackerProjectMilestone { id, name }` on `TrackerIssue`.
- **Future gateway entity**: `Milestone { id, project_id, linear_id, name, target_date, issues[], state }`.
- **ID rule**: Linear milestone UUID is the canonical ID. Gateway may wrap with a local stable ID in hosted mode.

### 1.3 `Issue`

A Linear issue under a milestone. An issue is a demoable vertical capability or deliverable unit.

- **Linear mapping**: `Linear Issue` with `parent_id: null` (top-level under milestone).
- **GSD-2 mapping**: GSD-2 slice.
- **OpenSymphony representation**: `NormalizedIssue` in the domain model.
- **Key fields**: `id` (Linear UUID), `identifier` (e.g. `COE-389`), `title`, `description`, `priority`, `state { id, name, category }`, `branch_name`, `url`, `labels`, `parent_id`, `blocked_by[]`, `sub_issues[]`, `created_at`, `updated_at`.
- **ID rule**: `IssueId` = Linear issue UUID string (e.g. `lin_389`). `IssueIdentifier` = human-readable ticket key (e.g. `COE-389`). Both are non-empty string wrappers.

### 1.4 `SubIssue`

A Linear sub-issue under an issue. A bounded unit of implementation, validation, documentation, or cleanup that can be assigned to a human or executed by an agent run.

- **Linear mapping**: `Linear Issue` with `parent_id` pointing to the parent issue.
- **GSD-2 mapping**: GSD-2 task.
- **OpenSymphony representation**: Same `NormalizedIssue` type; distinguished by `parent_id` being non-null.
- **In scheduling**: Sub-issues are leaf nodes that can be dispatched directly. Parent issues are blocked until all children reach terminal states.
- **ID rule**: Same as `Issue` — `IssueId` = Linear UUID, `IssueIdentifier` = ticket key.

### 1.5 `Run`

A specific OpenSymphony attempt to execute a Linear issue or sub-issue through a harness. Includes workspace, conversation/session, events, logs, terminal streams, diffs, validation, outcome, and retry metadata.

- **OpenSymphony representation**: `RunAttempt` in the domain model + `IssueExecution` state machine.
- **Lifecycle**: `Unclaimed → Claimed → Running → (RetryQueued | Released)`.
- **Key fields**: `worker_id`, `issue_id`, `issue_identifier`, `workspace_path`, `claimed_at`, `started_at`, `attempt` (`RetryAttempt`), `normal_retry_count`, `turn_count`, `max_turns`.
- **Outcome**: `WorkerOutcomeRecord` with `outcome` (`succeeded`, `failed`, `timed_out`, `stalled`, `cancelled`), `turn_count`, `summary`, `error`.
- **ID rule**: A run is identified by the tuple `(worker_id, issue_id, attempt)`. No standalone run UUID exists yet; runs are nested under `IssueExecution`.
- **Future gateway entity**: `Run { id, issue_id, attempt, worker_id, workspace_id, harness_session_id, state, started_at, finished_at, outcome, events[], diffs[], files[], validation_results[] }`.

### 1.6 `Workspace`

A deterministic per-issue filesystem workspace with lifecycle hooks (create, update, cleanup).

- **OpenSymphony representation**: `WorkspaceRecord`.
- **Key fields**: `path` (absolute filesystem path), `workspace_key` (sanitized issue identifier), `created_now`, `created_at`, `updated_at`, `last_seen_tracker_refresh_at`.
- **Sanitization**: `sanitize_workspace_key` replaces non-alphanumeric/`.`/`_`/` -` chars with `_`.
- **Local mode**: Physical directory under `workspace_root` (e.g. `/tmp/opensymphony/COE-389`).
- **Hosted mode**: Abstract workspace reference; actual path resolved by server-side isolation layer (container, VM, or managed sandbox).
- **ID rule**: `WorkspaceKey` = sanitized issue identifier. In hosted mode, a gateway-assigned `WorkspaceId` (UUID) will wrap the key.

### 1.7 `HarnessSession`

A live coding-agent conversation/session managed by an external harness (initially OpenHands agent-server).

- **OpenSymphony representation**: `ConversationMetadata` in the domain model.
- **Initial harness**: OpenHands agent-server.
- **Future harnesses**: Codex app-server, Rust-native SDK, in-process harnesses.
- **Key fields**: `conversation_id`, `server_base_url`, `transport_target`, `http_auth_mode`, `websocket_auth_mode`, `websocket_query_param_name`, `fresh_conversation`, `runtime_contract_version`, `stream_state`, `last_event_id`, `last_event_kind`, `last_event_at`, `last_event_summary`, `recent_activity[]`, `input_tokens`, `output_tokens`, `cache_read_tokens`, `total_tokens`, `runtime_seconds`.
- **Stream states**: `Detached → Attaching → Ready → Reconnecting → Closed | Failed`.
- **ID rule**: `ConversationId` = harness-assigned string (e.g. OpenHands conversation UUID). In hosted mode, gateway mints a `HarnessSessionId` that maps to the harness conversation.

### 1.8 `TerminalSession`

A bounded stream of terminal/log output produced by the harness during a run.

- **Current representation**: Not a standalone entity. Terminal output is part of `EventEnvelope` payloads (e.g. `ObservationEvent` with `tool_name: "terminal"`).
- **Future gateway entity**: `TerminalSession { id, run_id, harness_session_id, frames[], scrollback, state }`.
- **Stream behavior**: High-throughput, bounded, replayable with cursor. Separated from control events in the gateway stream design.
- **ID rule**: Derived from `Run.id + "terminal"` or gateway-assigned `TerminalSessionId`.

### 1.9 `PlanningSession`

A collaborative human-AI workspace for project intake, analysis, and task decomposition that produces Linear milestones, issues, and sub-issues.

- **Scope**: Out of scope for COE-389 implementation; vocabulary definition only.
- **Future gateway entity**: `PlanningSession { id, project_id, state, artifacts[], milestones_draft[], issues_draft[], human_approval_status, created_at, updated_at }`.
- **ID rule**: Gateway-assigned `PlanningSessionId` (UUID).

### 1.10 `Artifact`

A persistent output produced during a run or planning session: file changes, diffs, logs, validation results, screenshots, or planning documents.

- **Current representation**: `ControlPlaneFileChange { path, change_kind, lines_added, lines_removed }` in the control-plane snapshot.
- **Future gateway entity**: `Artifact { id, run_id, kind, path, url, size, created_at }`.
- **Kinds**: `file_change`, `diff`, `log`, `validation_result`, `screenshot`, `plan_document`.
- **ID rule**: Gateway-assigned `ArtifactId` (UUID). In local mode, physical path serves as implicit ID.

## 2. Linear-to-OpenSymphony ID Mapping

### 2.1 Local mode

| Linear Entity | OpenSymphony Domain Type | Mapping Rule |
|:--------------|:-------------------------|:-------------|
| `Project` (slugId) | implied by workflow | `project_slug` string in workflow config |
| `ProjectMilestone` (id, name) | `TrackerProjectMilestone` | Linear UUID → `id`, name passthrough |
| `Issue` (id, identifier) | `NormalizedIssue` | Linear UUID → `IssueId`, ticket key → `IssueIdentifier` |
| `Issue` parent_id | `NormalizedIssue.parent_id` | Linear parent UUID → `Option<IssueId>` |
| `Issue` state | `IssueState` | Linear state id → `TrackerStateId` (optional), name passthrough, category derived from workflow `active_states` / `terminal_states` |
| `Issue` sub_issues | `NormalizedIssue.sub_issues: Vec<IssueRef>` | Each child mapped to `IssueRef { id, identifier, state }` |
| `Issue` blocked_by | `NormalizedIssue.blocked_by: Vec<BlockerRef>` | Blocker id, identifier, state name |
| `Issue` branch_name | `NormalizedIssue.branch_name` | Linear branch_name (if set) or derived from identifier |

### 2.2 Hosted mode (future)

| OpenSymphony Entity | Mapping Rule |
|:--------------------|:-------------|
| `Project` | Gateway mints `ProjectId` (UUID). Maps to Linear `project_slug` via project configuration table. |
| `Milestone` | Gateway mints `MilestoneId` (UUID). Maps to Linear `ProjectMilestone.id`. |
| `Issue` / `SubIssue` | Reuses Linear `IssueId` (UUID) as primary key. `IssueIdentifier` remains human-readable. Gateway adds `ProjectId` and `MilestoneId` foreign keys for fast graph queries. |
| `Run` | Gateway mints `RunId` (UUID). Foreign keys: `issue_id`, `workspace_id`, `harness_session_id`. |
| `Workspace` | Gateway mints `WorkspaceId` (UUID). Maps to `WorkspaceKey` (sanitized issue identifier). Physical path is server-side private. |
| `HarnessSession` | Gateway mints `HarnessSessionId` (UUID). Maps to harness-specific `ConversationId`. |
| `TerminalSession` | Derived from `RunId` or gateway-minted `TerminalSessionId`. |
| `PlanningSession` | Gateway mints `PlanningSessionId` (UUID). Foreign key: `project_id`. |
| `Artifact` | Gateway mints `ArtifactId` (UUID). Foreign key: `run_id` or `planning_session_id`. URL is gateway-served. |

### 2.3 ID stability rules

1. **Linear IDs are immutable**: Once an issue is created in Linear, its UUID (`IssueId`) never changes. OpenSymphony treats this as the stable primary key.
2. **Issue identifiers are mutable in display**: Linear allows identifier reuse/reassignment. OpenSymphony uses `IssueId` for all internal references and `IssueIdentifier` only for human display and workspace key derivation.
3. **Workspace keys are derived from issue identifiers**: `WorkspaceKey::new(issue.identifier.as_str())`. If the identifier changes, the workspace key must be migrated or remapped.
4. **Runs are not globally unique yet**: In the current model, runs are scoped to `IssueExecution`. A future gateway will mint `RunId` UUIDs.
5. **Sequences, not IDs, for ordering**: `SnapshotEnvelope.sequence` and `EventEnvelope` timestamps provide ordering. Do not assume ID ordering.

## 3. Taxonomy consistency with PRD

The PRD (`docs/hosted-client-PRD.md`) defines the user-facing taxonomy. The mapping to current code is:

| PRD Concept | Current Code Type | Location |
|:------------|:------------------|:---------|
| `Project` | Implied by workflow `project_slug` | `opensymphony_workflow::ResolvedWorkflow` |
| `Milestone` | `TrackerProjectMilestone` | `opensymphony-domain::tracker::TrackerProjectMilestone` |
| `Issue` | `NormalizedIssue` | `opensymphony-domain::issue::NormalizedIssue` |
| `SubIssue` | `NormalizedIssue` with `parent_id` set | `opensymphony-domain::issue::NormalizedIssue` |
| `Run` | `RunAttempt` + `IssueExecution` | `opensymphony-domain::runtime::RunAttempt`, `opensymphony-domain::state_machine::IssueExecution` |
| `Workspace` | `WorkspaceRecord` | `opensymphony-domain::runtime::WorkspaceRecord` |
| `HarnessSession` | `ConversationMetadata` | `opensymphony-domain::runtime::ConversationMetadata` |
| `TerminalSession` | Part of `EventEnvelope` payload | `opensymphony-openhands::models::EventEnvelope` |
| `PlanningSession` | Not yet implemented | Future gateway entity |
| `Artifact` | `ControlPlaneFileChange` | `opensymphony-domain::control_plane::ControlPlaneFileChange` |

## 4. Event reference schema

Events that flow through the system use these entity references:

```text
ControlPlaneRecentEvent
├── happened_at: DateTime<Utc>
├── issue_identifier: Option<String> (e.g. "COE-389")
├── kind: enum
└── summary: String

ConversationActivityEvent
├── event_id: String
├── happened_at: TimestampMs
├── kind: String (harness event kind)
└── summary: String

EventEnvelope (OpenHands runtime)
├── id: String
├── timestamp: DateTime<Utc>
├── source: String
├── kind: String
├── payload: serde_json::Value
├── key: Option<String>
└── value: Option<serde_json::Value>
```

Future gateway event journal will add:
- `sequence: u64` (global monotonic sequence)
- `correlation_id: String` (links action request to resulting events)
- `entity_ref: EntityRef { kind, id }`
