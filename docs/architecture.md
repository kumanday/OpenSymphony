# Architecture

## 1. Objective

Implement the Symphony design in Rust while using OpenHands agent-server as the execution substrate and FrankenTUI as an optional terminal operator client.

The architecture must preserve the Symphony boundaries:

- the orchestrator is the source of truth for scheduling state
- the tracker is polled and reconciled by the orchestrator
- each issue executes in its own workspace path
- `WORKFLOW.md` remains the repo-owned policy and prompt contract
- UI is optional and must not affect correctness

## 2. Design summary

OpenSymphony is split into five layers:

1. Policy layer
   - `WORKFLOW.md`
   - repository-owned `AGENTS.md` and `.agents/skills/` inside the target repository
2. Configuration layer
   - typed workflow/config loader
   - env resolution
   - `openhands` extension config
3. Coordination layer
   - orchestrator actor
   - retry queue
   - reconciliation
   - runtime snapshot store
4. Execution layer
   - workspace manager
   - OpenHands REST client
   - OpenHands WebSocket runtime stream
   - issue session runner
5. Observability layer
   - structured logs
   - control-plane API
   - FrankenTUI client

## 3. Main architectural decisions

### 3.1 Rust owns orchestration

The Rust daemon owns all scheduler semantics that Symphony specifies:

- poll cadence
- issue eligibility
- bounded concurrency
- claim/release state
- retry scheduling
- stall detection
- startup cleanup
- restart recovery
- operator snapshots

OpenHands conversation state is informative, not authoritative for scheduling.

Hierarchy-aware task selection is part of issue eligibility. Parent issues stay blocked until every child issue in the latest tracker snapshot is terminal, and ready leaf issues sort ahead of ready parents within the same priority bucket.

### 3.2 OpenHands agent-server is an execution adapter

OpenHands is used as the agent execution backend because it exposes:

- per-conversation workspace configuration
- persistent conversations
- background run triggering
- searchable event history
- real-time state updates over WebSocket
- provider/model flexibility
- tools and MCP

OpenSymphony does not reimplement an agent loop. It adapts Symphony's worker lifecycle onto OpenHands conversations.

### 3.3 WebSocket-first, not WebSocket-only

Agent-session updates use a WebSocket-first design from the beginning.

REST remains necessary for:

- conversation creation
- sending user events
- triggering runs
- reading authoritative conversation info
- initial event sync
- post-reconnect reconciliation
- recovery after missed events or process restarts

This avoids a throwaway polling-only runtime client, while still being robust to dropped WebSocket connections.

### 3.4 One local server, many issue workspaces

For the local MVP, one local OpenHands agent-server subprocess is shared across issues. Each issue still gets its own workspace and passes that path as the conversation `working_dir`.

This avoids one Docker container per workspace while preserving the per-issue filesystem boundary required by Symphony.

### 3.5 One conversation per issue by default

OpenSymphony creates a stable OpenHands `conversation_id` per issue and persists it inside the issue workspace. That conversation is reused across:

- multiple turns within one worker lifetime
- normal continuation retries after a worker exits
- restart recovery after daemon restarts

This is stricter than the minimum Symphony requirement and intentionally optimizes continuity.
The current issue session runner also tracks whether that conversation has already been seeded with the full workflow prompt so a reused but never-started thread can still receive the original assignment on the next attempt.
The persisted OpenHands state directory is derived from the workflow-owned `openhands.conversation.persistence_dir_relative` path inside the issue workspace, and a missing-but-recreatable conversation that still has persisted history stays on continuation guidance instead of replaying the full workflow template.
The persisted conversation manifest also records the owning issue reference, creation and attach timestamps, and the launch profile used to create the thread so `opensymphony debug <issue-id>` can reattach to the same session or rehydrate the same `conversation_id` with matching runtime settings.

### 3.6 The UI only sees the control plane

FrankenTUI attaches to the OpenSymphony control-plane API, not directly to OpenHands and not directly to orchestrator internals.

This keeps:

- daemon correctness independent from UI
- UI crashes from affecting execution
- future hosted deployment options open

## 4. Runtime component model

## 4.1 Core crates

Current crate boundaries:

- `opensymphony-domain`
  - issue model
  - run-attempt model
  - retry-entry model
  - scheduler state and transition types
  - orchestrator snapshot model
- `opensymphony-workflow`
  - `WORKFLOW.md` loader
  - YAML front matter parsing
  - strict prompt rendering
  - config validation plus defaults/env/path resolution
  - OpenHands extension config kept separate from core workflow config
- `opensymphony-workspace`
  - workspace mapping
  - sanitization and containment
  - hook runner
  - issue, run, and conversation manifests
  - prompt capture helpers and generated issue/session context artifacts
  - root-scoped `after_create` bootstrap receipt for post-hook recovery
  - manifest-backed workspace ownership checks for colliding sanitized keys
  - symlink rejection for reused workspace roots
  - managed metadata path safety for `.opensymphony/`
  - process-tree teardown for timed-out hooks
- `opensymphony-linear`
  - Linear GraphQL client
  - issue normalization
  - candidate fetching
  - state reconciliation
- `opensymphony-linear-mcp`
  - line-delimited stdio MCP server for agent-side ticket writes
  - `initialize`, `ping`, `tools/list`, and `tools/call`
  - minimal Linear tool surface backed by direct GraphQL mutations
- `opensymphony-openhands`
  - repo-local tooling resolution
  - local server supervisor
  - REST client
  - WebSocket stream
  - event cache
  - issue session runner
- `opensymphony-orchestrator`
  - poll tick and long-running scheduler loop
  - generic `Scheduler<TTracker, TWorkspace, TWorker>` core over tracker, workspace, and worker backends
  - worker registry plus worker-report ingestion
  - retry timers, stall handling, and state reconciliation
  - manifest-backed restart recovery for workspace reuse
- `opensymphony-control`
  - snapshot store
  - local HTTP and WebSocket control-plane API
- `opensymphony-cli`
  - `run` orchestrator startup
  - `daemon` demo control-plane startup
  - doctor command
  - target-repo `WORKFLOW.md` resolution and prompt preflight for doctor
  - repo-root OpenHands preflight checks
  - linear-mcp command
  - config entrypoints
- `opensymphony-tui`
  - FrankenTUI client over control plane
- `opensymphony-testkit`
  - fake Linear server helpers
  - fake OpenHands agent-server
  - shared fixtures

## 4.1.1 M1 public contract surface

The repository now exposes three stable foundation contracts that later milestones build on:

- `opensymphony-domain`
  - normalized tracker issue model
  - blocker references
  - parent/sub-issue references for hierarchy-aware dispatch
  - run-attempt, retry-entry, runtime-session, and worker-outcome models
  - serialized orchestrator snapshot types
- `opensymphony-workflow`
  - raw `WORKFLOW.md` parsing into `{config, prompt_template}`
  - typed config resolution with fail-fast unknown nested workflow keys plus fail-fast unknown top-level keys outside the supported opaque `codex` namespace, defaults, env indirection, path normalization, required Linear tracker credentials via either explicit `tracker.api_key` or process-level `LINEAR_API_KEY`, explicit env-backed workspace roots, and strict `openhands` extension validation
  - strict prompt rendering over `{issue, attempt}`
- `opensymphony-orchestrator`
  - deterministic candidate sorting with leaf-before-parent ordering
  - blocker-aware and hierarchy-aware dispatch eligibility helpers
  - generic scheduler configuration sourced from workflow polling, concurrency, retry, and stall settings
  - explicit `Claimed` / `Running` / `RetryQueued` / `Released` transitions driven only by orchestrator-owned commands and worker reports
  - fixed continuation retry, exponential failure backoff, bounded global/per-state capacity via scheduler-owned cached running counts, running-worker reconciliation, terminal cleanup, and manifest-backed restart recovery for workspace reuse

The other crates are already present at their final ownership boundaries, but for M1 they intentionally expose only thin re-exports or placeholders rather than premature transport logic.

## 4.2 External processes

Local MVP process graph:

- `opensymphony run`
  - owns orchestrator and control plane
  - may spawn:
    - `bash tools/openhands-server/run-local.sh`
- `opensymphony debug <issue-id>`
  - reuses the issue workspace and persisted conversation manifest
  - attaches to the configured OpenHands transport directly or reuses a ready local supervised server on the same base URL
- OpenHands MCP child processes
  - may spawn:
    - `opensymphony linear-mcp`
  - write ticket comments, transitions, and links directly to Linear
  - do not participate in scheduler correctness
- `opensymphony tui`
  - separate process
  - reads control-plane APIs only
- target-repo issue workspace hooks
  - started by workspace manager
- OpenHands agent subprocesses or tool execution
  - managed by agent-server

The current local supervisor implementation resolves its launch metadata from
`tools/openhands-server/`, probes readiness with `GET /openapi.json`, and only
terminates a process that it launched itself. In supervised mode it also refuses
to launch when another ready server is already responding on the configured
base URL, so the daemon never silently adopts a foreign process as its owned
child. Workflow resolution now accepts absolute `http://` and `https://`
OpenHands origins with optional path prefixes, rejects embedded credentials plus
query/fragment suffixes, and still rejects bracketed IPv6 until the local
readiness probe grows that support. Non-loopback targets must use `https://`
and configure `openhands.transport.session_api_key_env`. The runtime attach
loop now consumes workflow-owned WebSocket readiness and reconnect budgets.
The runtime also now accepts workflow-owned `openhands.local_server.command`
overrides for managed local supervision and resolves workflow-owned
`agent.llm.api_key_env` and `agent.llm.base_url_env` names when building the
conversation-create payload. Explicit `local_server.enabled`, `local_server.env`,
`local_server.readiness_probe_path`, `local_server.startup_timeout_ms`, and
`websocket.enabled` remain rejected during workflow resolution until the local
supervisor and readiness path can honor them end to end.

## 5. Worker and conversation model

This is the most important mapping in the whole system.

### 5.1 Terminology

- Issue: a Linear issue normalized by the tracker adapter
- Worker lifetime: one orchestrator dispatch of one issue
- Turn: one prompt plus one OpenHands `run` cycle within a live conversation
- Conversation: one persistent OpenHands remote conversation, reused across worker lifetimes by default

### 5.2 Why keep worker and conversation separate

Symphony explicitly allows multiple turns inside one worker and continuation retries after normal worker exit. OpenHands exposes persistent conversations that can survive beyond one run trigger.

Mapping them separately preserves both models:

- worker lifetime remains a Symphony scheduler concept
- conversation lifetime becomes an execution-memory concept

### 5.3 Default issue run policy

For each issue:

1. Ensure workspace exists.
2. Load or create stable conversation metadata.
3. Attach WebSocket stream and reconcile events.
4. If that reused conversation is already `queued` or `running`, wait for it to
   reach a terminal state before sending the next prompt.
5. Execute one or more turns on the same conversation up to `agent.max_turns`.
   If `POST /run` races with an already-active turn and returns `409 Conflict`,
   wait for that active turn to finish, refresh the event backlog, and retry the
   run on the same conversation.
6. Exit worker normally or abnormally.
7. Let the orchestrator decide continuation retry, failure retry, release, or cancellation.

### 5.4 Prompt policy

Use different prompt shapes for different moments:

- Fresh conversation, first turn:
  - full rendered workflow prompt
- Existing conversation whose full workflow prompt was never successfully sent:
  - full rendered workflow prompt
- Existing conversation, first turn of a new worker lifetime:
  - continuation guidance only
- Existing conversation, in-process turn 2..N:
  - continuation guidance only
- Conversation reset after corruption or incompatible version:
  - fresh full workflow prompt again

The current runner implements continuation guidance as a small built-in resume message that tells the agent to keep working from the existing conversation and workspace context instead of replaying the workflow template.
This follows Symphony's instruction not to resend the original full task prompt into an already live thread.

## 6. Event and state ownership

## 6.1 OpenHands events

OpenHands generates detailed runtime events. OpenSymphony uses them for:

- progress visibility
- cached conversation state
- last-seen event timestamp
- stall detection inputs
- terminal-status detection
- debugging artifacts

OpenSymphony must not rely on an exact closed set of event kinds. Unknown events are retained as raw payloads.

## 6.2 Orchestrator snapshot

The control plane exposes a summarized runtime snapshot derived from:

- orchestrator running map
- retry queue
- tracker state refreshes
- aggregated token and usage totals
- recent OpenHands event summaries
- local agent-server health
- per-issue OpenHands server base URL plus transport and auth diagnostics

The snapshot is not just a projection of OpenHands state. It is a Symphony-specific view.

## 6.3 Implemented observability slice

The current repository implements the first read-only control-plane and FrankenTUI slice with these concrete boundaries:

- `opensymphony-domain`
  - `SnapshotEnvelope`
  - daemon, issue, metrics, and recent-event serialization models, including
    `server_base_url`, `transport_target`, `http_auth_mode`,
    `websocket_auth_mode`, and `websocket_query_param_name`
- `opensymphony-control`
  - in-memory snapshot store
  - `GET /healthz`
  - `GET /api/v1/snapshot`
  - `GET /api/v1/events` using Server-Sent Events with lagged-subscriber catch-up to the newest snapshot
- `opensymphony-tui`
  - reducer-owned TUI state
  - REST bootstrap plus SSE reconnect loop
  - latest-value bridge mailbox that coalesces bursty snapshots
  - reconnect indicator that preserves the last good snapshot while the bridge resubscribes
  - inline-mode rendering over immutable view text with pane-specific row budgeting
- `opensymphony-cli`
  - `daemon` and `tui` entrypoints used for local attach and detach validation

This slice deliberately keeps the UI on a stable read-only contract while the orchestration crates continue to mature behind it.

## 7. Local MVP data flow

## 7.1 Dispatch path

1. Poll tick fires.
2. Orchestrator reconciles running issues, retry queue, and claimed-only reservations.
3. Orchestrator fetches candidate issues from Linear.
4. Orchestrator selects eligible issues subject to concurrency.
5. Worker starts for one issue.
6. Workspace manager creates or reuses issue workspace.
7. `before_run` hook executes.
8. OpenHands runtime loads or creates conversation.
9. OpenHands runtime attaches WebSocket stream and reconciles event history.
10. Prompt is chosen and sent as a user event.
11. OpenHands run is triggered.
12. Runtime events stream back over WebSocket and are mirrored into state, logs, and scheduler worker reports.
13. Worker decides whether to do another in-process turn.
14. Worker backend reports runtime progress and the terminal worker outcome to the scheduler.
15. Orchestrator schedules a continuation retry, failure retry, release, or cleanup.

## 7.2 ASCII sequence

```text
PollTick
  -> LinearAdapter.fetch_candidates()
  -> Orchestrator.dispatch(issue)
     -> WorkspaceManager.ensure(issue)
     -> HookRunner.before_run()
     -> OpenHandsRunner.attach_or_create(issue)
        -> REST POST /api/conversations
        -> WS /sockets/events/{conversation_id}
        -> REST GET /events/search
     -> OpenHandsRunner.send_turn(prompt)
        -> REST POST /events
        -> REST POST /run
        -> WS stream events
     -> WorkerOutcome
  -> Orchestrator.schedule_next_state()
  -> SnapshotPublisher.publish()
```

## 7.3 Current local attach path

The implemented local attach path now uses the real orchestrator entrypoint while preserving the same boundary:

1. `opensymphony-cli run` starts the orchestrator and local control-plane server.
2. A snapshot store publishes immutable `SnapshotEnvelope` values.
3. `opensymphony-cli tui` fetches `/api/v1/snapshot` and renders it as bootstrap state.
4. The TUI gives that snapshot fetch a bounded timeout, then opens `/api/v1/events` behind a separate bounded stream-attach watchdog that stays armed until the first snapshot arrives. That deadline is measured across the whole pre-snapshot phase rather than restarting on keepalive comments or other non-snapshot SSE frames, keeps `conn=connecting` until the stream delivers that first snapshot, and publishes the first streamed snapshot plus the live attachment signal atomically before listening for ongoing SSE updates.
5. If an SSE client lags, the control plane immediately fast-forwards it to `store.current()` instead of waiting for the retained broadcast backlog to drain, and suppresses any older retained sequences so reducers never regress.
6. On snapshot or stream failure, the TUI keeps rendering the last good snapshot, marks the connection as reconnecting, then retries the current snapshot fetch before resubscribing.
7. Detaching the UI leaves the daemon process and snapshot publication unaffected.

The separate `opensymphony-cli daemon` command remains available only as a demo snapshot publisher for smoke tests and UI-focused development.

## 8. Recovery model

## 8.1 Daemon restart

On startup:

- validate config
- initialize control-plane state
- clean up terminal-state workspaces if configured
- load known issue metadata from workspace manifests if present
- reuse recovered workspace attachments for still-active issues on the next scheduler poll
- rebuild retry queue from persisted retry metadata if implemented in a later milestone
- treat OpenHands conversations as attachable resources, not as scheduler truth

If a conversation exists but its issue is no longer active, the orchestrator does not resume it.

Current repository implementation:

- `opensymphony-orchestrator::Scheduler` recovers workspace ownership from manifest-derived `RecoveryRecord` entries and uses tracker state to decide whether recovered work should be redispatched, retained as inactive, or cleaned up as terminal
- retry scheduling itself is still derived in memory from live worker outcomes rather than a separately persisted retry journal

## 8.2 WebSocket disconnect

On disconnect:

- mark runtime stream as degraded
- start bounded reconnect backoff
- refresh authoritative conversation state with REST
- reconnect WebSocket
- wait for readiness barrier
- reconcile event backlog
- resume live processing

If reconnect cannot recover within policy limits, fail the worker and schedule a Symphony retry.

## 8.3 Agent-server restart

If the local server dies:

- mark all active workers as transport-failed
- stop sending new turns
- attempt one coordinated server restart if allowed by policy
- workers may either reattach to persisted conversations or fail and rely on orchestrator retry

The orchestrator should stay alive when the local agent-server crashes.

## 9. Local MVP boundaries

The local MVP intentionally assumes a trusted machine and trusted repositories.

Tradeoffs:

- fast and simple local setup
- no per-issue Docker RAM overhead
- no host isolation guarantees
- stronger need for clear documentation and safe defaults

Hosted and hardened modes are described in `docs/deployment-modes.md` but are not the critical path for the MVP.

## 10. Non-goals for the architecture phase

- implementing a multi-tenant hosted service first
- direct Rust implementation of OpenHands tool execution
- UI-driven orchestration
- reusing OpenHands web-app protocols
- pushing tracker orchestration into MCP tools
