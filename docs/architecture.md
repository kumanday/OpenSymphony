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
  - orchestrator snapshot model
- `opensymphony-workflow`
  - `WORKFLOW.md` loader
  - YAML front matter parsing
  - strict prompt rendering
  - config validation
- `opensymphony-workspace`
  - workspace mapping
  - sanitization and containment
  - hook runner
  - issue metadata manifest
- `opensymphony-linear`
  - Linear GraphQL client
  - issue normalization
  - candidate fetching
  - state reconciliation
- `opensymphony-linear-mcp`
  - stdio MCP server for agent-side ticket writes
- `opensymphony-openhands`
  - local server supervisor
  - REST client
  - WebSocket stream
  - event cache
  - issue session runner
- `opensymphony-orchestrator`
  - poll tick
  - runtime state machine
  - worker supervision
  - retry timers
  - reconciliation
- `opensymphony-control`
  - snapshot store
  - local HTTP and WebSocket control-plane API
- `opensymphony-cli`
  - daemon startup
  - doctor command
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
  - run-attempt, retry-entry, runtime-session, and worker-outcome models
  - serialized orchestrator snapshot types
- `opensymphony-workflow`
  - raw `WORKFLOW.md` parsing into `{config, prompt_template}`
  - typed config resolution with defaults, env indirection, path normalization, and `openhands` extension validation
  - strict prompt rendering over `{issue, attempt}`
- `opensymphony-orchestrator`
  - deterministic candidate sorting and claim logic
  - explicit `Running` / `RetryQueued` / `Released` transitions
  - fixed continuation retry, exponential failure backoff, stall detection, reconciliation, and restart recovery

The other crates are already present at their final ownership boundaries, but for M1 they intentionally expose only thin re-exports or placeholders rather than premature transport logic.

## 4.2 External processes

Local MVP process graph:

- `opensymphony daemon`
  - owns orchestrator and control plane
  - may spawn:
    - `python -m openhands.agent_server`
- `opensymphony tui`
  - separate process
  - reads control-plane APIs only
- target-repo issue workspace hooks
  - started by workspace manager
- OpenHands agent subprocesses or tool execution
  - managed by agent-server

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
4. Execute one or more turns on the same conversation up to `agent.max_turns`.
5. Exit worker normally or abnormally.
6. Let the orchestrator decide continuation retry, failure retry, release, or cancellation.

### 5.4 Prompt policy

Use different prompt shapes for different moments:

- Fresh conversation, first turn:
  - full rendered workflow prompt
- Existing conversation, first turn of a new worker lifetime:
  - continuation guidance only
- Existing conversation, in-process turn 2..N:
  - continuation guidance only
- Conversation reset after corruption or incompatible version:
  - fresh full workflow prompt again

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

The snapshot is not just a projection of OpenHands state. It is a Symphony-specific view.

## 7. Local MVP data flow

## 7.1 Dispatch path

1. Poll tick fires.
2. Orchestrator reconciles running issues and retry queue.
3. Orchestrator fetches candidate issues from Linear.
4. Orchestrator selects eligible issues subject to concurrency.
5. Worker starts for one issue.
6. Workspace manager creates or reuses issue workspace.
7. `before_run` hook executes.
8. OpenHands runtime loads or creates conversation.
9. OpenHands runtime attaches WebSocket stream and reconciles event history.
10. Prompt is chosen and sent as a user event.
11. OpenHands run is triggered.
12. Runtime events stream back over WebSocket and are mirrored into state and logs.
13. Worker decides whether to do another in-process turn.
14. Worker exits and reports success, failure, timeout, stall, or cancellation.
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

## 8. Recovery model

## 8.1 Daemon restart

On startup:

- validate config
- initialize control-plane state
- clean up terminal-state workspaces if configured
- load known issue metadata from workspace manifests if present
- rebuild retry queue from persisted retry metadata if implemented
- treat OpenHands conversations as attachable resources, not as scheduler truth

If a conversation exists but its issue is no longer active, the orchestrator does not resume it.

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
