# OpenSymphony Rich Client, Hosted Mode, and Planning Implementation Plan

## 1. Implementation strategy

The implementation should proceed from contracts to clients to hosted execution. The first priority is to create a stable OpenSymphony Gateway that rich clients can use without depending on private orchestrator internals. The second priority is to build a shared frontend that works in Tauri desktop and browser contexts. The third priority is to deepen the Linear task graph and planning workflow. Hosted mode and future harness/auth integrations should then build on those contracts.

The main sequencing rule is:

1. Make server state and streams reliable.
2. Build clients against versioned APIs.
3. Add mutations only where server-side intent handling is clear.
4. Add hosted auth and isolation after local/external host-client contracts are stable.
5. Add future Codex/OAuth integrations after the harness and auth seams are proven.

## 2. Dependency overview

```text
P0 Discovery and contracts
  ├─ P1 Gateway foundation
  │  ├─ P2 Shared frontend foundation
  │  │  ├─ P3 Desktop local client
  │  │  ├─ P4 Task graph UI and Linear mutations
  │  │  ├─ P5 OpenHands rich runtime views
  │  │  └─ P6 Collaborative planning flow
  │  └─ P7 Web client
  ├─ P8 Hosted mode alpha
  └─ P9 Future harness/auth readiness

P10 Hardening and release can run across all phases after P1 begins.
```

## 3. Phase P0: Discovery, baseline contracts, and feasibility gates

### Objective

Establish the technical baseline, versioned contracts, and integration risks before building the client-facing features.

### Dependencies

None.

### Tasks

#### P0.1 Inventory current OpenSymphony control plane

- Catalog current health, snapshot, and event endpoints.
- Catalog current snapshot envelope structure.
- Catalog current OpenHands runtime event handling.
- Catalog current Linear GraphQL helper/query assets.
- Catalog current CLI commands and operator flows.
- Identify private orchestrator structs that must not leak into public client contracts.

Output:

- Current API inventory.
- Public/private boundary notes.
- Migration list for read-only control plane to versioned gateway.

#### P0.2 Define domain vocabulary and IDs

- Define `Project`, `Milestone`, `Issue`, `SubIssue`, `Run`, `Workspace`, `HarnessSession`, `TerminalSession`, `PlanningSession`, and `Artifact` IDs.
- Define mapping from Linear entities to OpenSymphony entities.
- Define stable ID behavior for local mode and hosted mode.
- Define entity references for events.

Output:

- Domain vocabulary document.
- ID mapping rules.
- Entity reference schema.

#### P0.3 Draft gateway schemas

- Draft JSON schemas or Rust structs for snapshots, events, task graph nodes, run details, terminal frames, approval requests, planning artifacts, and capabilities.
- Include schema version fields.
- Include cursor and sequence fields.
- Include unknown payload preservation fields.

Output:

- `gateway-schema-v1` draft.
- Schema review checklist.

#### P0.4 Benchmark stream options

- Benchmark current SSE snapshot stream under bursty updates.
- Benchmark WebSocket control event stream.
- Benchmark binary terminal/log frame delivery through browser WebSocket.
- Benchmark JSON-RPC 2.0 over WebSocket as a hosted bidirectional control envelope.
- Benchmark Tauri channel delivery for desktop local mode.
- Benchmark local native IPC options for a separate local daemon: Unix domain sockets on macOS/Linux and named pipes on Windows.
- Benchmark in-process Rust channel delivery for an embedded or directly attached local host.
- Measure UI decode and render cost for representative logs and terminal output.

Output:

- Stream benchmark report.
- Recommended stream split: control events, terminal/log frames, detail reads, and optional JSON-RPC control sessions.
- Recommended desktop local transport order: in-process Rust channels, native IPC, Tauri channels, loopback HTTP/WebSocket fallback.
- Initial throughput and latency targets.

#### P0.5 Validate Tauri security and native integration plan

- Define Tauri capabilities by window and command.
- Validate OS keychain integration path.
- Validate local daemon discovery and supervision model.
- Validate native file/folder picker flow.
- Validate notification flow.

Output:

- Tauri shell capability matrix.
- Desktop native integration plan.

#### P0.6 Validate OpenHands contract and version pinning

- Confirm pinned OpenHands agent-server version used by OpenSymphony.
- Confirm HTTP endpoints needed for create, send, run, search events, and recovery.
- Confirm WebSocket path, auth modes, readiness behavior, and event types.
- Confirm unknown-event handling and raw payload preservation.
- Create a contract fixture set from fake and live server behavior.

Output:

- OpenHands contract matrix.
- Fake server fixtures.
- Live pinned server test plan.

#### P0.7 Future Codex app-server spike

This is discovery only, not production implementation.

- Confirm current Codex app-server transport modes.
- Confirm JSON-RPC schema generation commands.
- Confirm stdio startup behavior.
- Confirm WebSocket auth flags and limitations.
- Sketch `CodexHarnessAdapter` interface compatibility with OpenHands adapter.
- Identify approval, thread, turn, and event mapping needs.

Output:

- Codex future integration memo.
- Harness abstraction adjustments.
- Feature-gate recommendation.

#### P0.8 Subscription credentials and model configuration spike

This is discovery only, not production implementation.

- Confirm OpenHands SDK `LLM.subscription_login` behavior for subscription-backed auth, starting with OpenAI ChatGPT/Codex.
- Confirm how subscription-backed `LLM` objects attach to OpenHands `Agent` and `Conversation` with `openhands agent-server`.
- Confirm which documented OpenAI/Codex client flows are relevant for future Codex app-server use.
- Confirm the existing API-compatible configuration behavior for `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`.
- Define model configuration metadata for base URL, model string, credential reference, and harness capability.
- Define a provider-specific subscription credential adapter interface.
- Define local credential storage requirements.
- Define hosted credential storage requirements.
- Define refresh-token ownership, access-token injection, and redaction requirements.
- Define validation and display behavior for configured model settings.

Output:

- Subscription credential integration memo.
- Model and credential settings draft.
- Model configuration metadata draft.

#### P0.9 Hosted mode threat model

- Identify hosted trust boundaries.
- Identify tenant isolation requirements.
- Identify workspace isolation options.
- Identify secrets boundaries.
- Identify audit requirements.
- Identify resource quota needs.

Output:

- Hosted mode threat model.
- Hosted deployment risk register.

### Exit criteria

- Gateway v1 draft schemas approved.
- Stream strategy chosen.
- Harness abstraction can support OpenHands now and Codex later.
- Desktop and web transport adapter requirements defined.
- Hosted mode security constraints documented.

## 4. Phase P1: OpenSymphony Gateway foundation

### Objective

Convert the existing control plane into a versioned gateway that supports rich clients through snapshots, details, streams, capabilities, and action receipts.

### Dependencies

P0.1, P0.2, P0.3, P0.4, P0.6.

### Tasks

#### P1.1 Create gateway module boundary

- Add or reorganize the server module into `opensymphony_gateway` or equivalent internal module tree.
- Keep internal orchestrator mutation APIs private.
- Expose only versioned DTOs and action handlers.
- Add API version metadata.

Depends on: P0.1, P0.3.

Output:

- Gateway module skeleton.
- Public DTO crate/module.
- Version constant.

#### P1.2 Implement capability discovery

- Add `/api/v1/capabilities`.
- Include gateway version.
- Include stream modes.
- Include available actions.
- Include harness adapters.
- Include tracker adapters.
- Include planning availability.
- Include experimental future integration flags.

Depends on: P1.1.

Output:

- Capability endpoint.
- Capability tests.

#### P1.3 Implement dashboard snapshot v1

- Convert existing snapshot into stable public schema.
- Include daemon health, Linear sync health, harness health, active runs, queue, retries, and recent events.
- Add sequence number and snapshot timestamp.

Depends on: P1.1.

Output:

- `/api/v1/dashboard/snapshot`.
- Snapshot fixture tests.

#### P1.4 Implement task graph read API

- Add project list endpoint.
- Add project detail endpoint.
- Add task graph endpoint.
- Include milestones, issues, sub-issues, and runtime overlays.
- Add pagination or lazy detail where needed.

Depends on: P1.1, P0.2.

Output:

- `/api/v1/projects`.
- `/api/v1/projects/{id}`.
- `/api/v1/projects/{id}/taskgraph`.
- Task graph fixtures.

#### P1.5 Implement run detail API

- Add run detail endpoint.
- Include issue/sub-issue context, workspace, harness session, lifecycle state, events summary, diff summary, validation summary, and action capabilities.
- Add event history endpoint with cursor.

Depends on: P1.1, P1.3.

Output:

- `/api/v1/runs/{run_id}`.
- `/api/v1/runs/{run_id}/events`.
- Run detail fixtures.

#### P1.6 Implement file and diff summary API

- Add changed-files endpoint.
- Add per-file diff endpoint.
- Add raw artifact references where needed.
- Ensure hosted mode can later abstract physical paths.

Depends on: P1.5.

Output:

- `/api/v1/runs/{run_id}/files`.
- `/api/v1/runs/{run_id}/diffs`.
- File safety tests.

#### P1.7 Implement event journal v1

- Add durable event records with sequence numbers.
- Record orchestrator events.
- Record gateway action events.
- Record normalized harness events.
- Preserve raw harness payload references.
- Support cursor reads.

Depends on: P1.1, P0.3.

Output:

- Event journal storage.
- Cursor query API.
- Event schema tests.

#### P1.8 Implement stream broker v1

- Stream events by cursor.
- Support reconnect.
- Support bounded queues.
- Support latest-snapshot coalescing for dashboard events.
- Expose connection state and stream errors.

Depends on: P1.7, P0.4.

Output:

- `/api/v1/events?cursor=`.
- Optional WebSocket event stream.
- Stream reconnect tests.

#### P1.9 Implement action receipt framework

- Define action request envelope.
- Define action receipt envelope.
- Add correlation IDs.
- Record accepted/rejected action events.
- Add permission placeholder for hosted mode.

Depends on: P1.7.

Output:

- Action envelope types.
- Action audit records.

#### P1.10 Add initial actions

Implement safe initial actions:

- Retry run.
- Cancel run where supported.
- Rehydrate conversation.
- Add issue comment.
- Open debug session metadata.

Depends on: P1.9, existing orchestrator operations.

Output:

- Initial action endpoints.
- Action tests.

### Exit criteria

- Rich clients can consume all required read-only state through `/api/v1`.
- Streams support cursor replay.
- Initial action intents return receipts and publish correlated events.
- Existing CLI and TUI behavior remains intact.

## 5. Phase P2: Shared frontend foundation

### Objective

Create the shared TypeScript frontend architecture used by both Tauri desktop and web clients.

### Dependencies

P1.2, P1.3, P1.4, P1.5, P1.8.

### Tasks

#### P2.1 Create frontend workspace

- Set up monorepo package structure.
- Add shared TypeScript build tooling.
- Add UI package.
- Add state package.
- Add API client package.
- Add terminal renderer package.
- Add testing setup.

Depends on: P1 schemas.

Output:

- Frontend workspace skeleton.
- Build and test commands.

#### P2.2 Generate or define TypeScript schemas

- Generate TypeScript types from gateway schemas or maintain schema-aligned types.
- Add schema version constants.
- Add runtime validation for stream payloads where needed.

Depends on: P2.1, P1 DTOs.

Output:

- Typed API models.
- Schema compatibility tests.

#### P2.3 Implement gateway API client

- Implement REST client.
- Implement event stream client.
- Implement binary stream placeholder.
- Implement action mutation client.
- Define transport adapter contracts for desktop local native, desktop local gateway, desktop remote gateway, browser gateway, and tests.
- Implement error and reconnect handling.

Depends on: P2.2.

Output:

- `api-client` package.
- Mock transport.
- Integration tests against gateway fixtures.

#### P2.4 Implement reducer state model

- Create entity cache.
- Create dashboard reducer.
- Create task graph reducer.
- Create run reducer.
- Create stream state reducer.
- Create planning reducer placeholder.
- Add stale-data and reconnect states.

Depends on: P2.3.

Output:

- `state` package.
- Reducer tests.

#### P2.5 Implement app shell layout

- Add navigation shell.
- Add project sidebar.
- Add resizable panes.
- Add command palette placeholder.
- Add connection status bar.
- Add keyboard focus model.

Depends on: P2.1.

Output:

- Shared layout components.
- UI smoke tests.

#### P2.6 Implement dashboard view

- Render gateway health.
- Render active runs, queue, retries, and recent events.
- Link dashboard rows to projects and runs.
- Show stale/reconnecting state.

Depends on: P2.4, P2.5.

Output:

- Dashboard page.
- Dashboard fixture tests.

#### P2.7 Implement task graph read view

- Render project tree.
- Render milestone/issue/sub-issue hierarchy.
- Render runtime overlay badges.
- Add filters and search.
- Link issues and sub-issues to run detail.

Depends on: P2.4, P2.5.

Output:

- Task graph explorer.
- Task graph tests.

#### P2.8 Implement run detail read view

- Render run summary.
- Render event timeline.
- Render workspace and harness metadata.
- Render action capability bar.
- Render diff and validation placeholders.

Depends on: P2.4, P2.5.

Output:

- Run detail page.
- Run detail fixture tests.

#### P2.9 Implement terminal/log renderer prototype

- Implement worker-based decoder/render loop.
- Support text log frames first.
- Add terminal cell delta frame support if available.
- Add scrollback, search, copy, jump-to-latest.
- Measure frame rate and UI responsiveness.

Depends on: P0.4, P2.3.

Output:

- Terminal/log pane prototype.
- Renderer benchmark harness.

### Exit criteria

- Shared frontend can render dashboard, task graph, and run details from gateway fixtures.
- Stream reconnect behavior is visible in UI state.
- Terminal/log pane prototype can handle representative bursty output.

## 6. Phase P3: Tauri desktop local client

### Objective

Package the shared frontend as a Tauri desktop app with native local integrations and high-performance local operation.

### Dependencies

P2.1 through P2.9, P0.5.

### Tasks

#### P3.1 Create Tauri desktop app

- Create Tauri project wrapper.
- Mount shared frontend.
- Configure development and production builds.
- Add app metadata and icons.

Depends on: P2.1.

Output:

- Desktop app skeleton.
- Build script.

#### P3.2 Implement desktop connection profiles

- Add local daemon profile.
- Add supervised local daemon profile.
- Add embedded or directly attached local host profile if the packaging decision supports it.
- Add local native IPC profile for a separate trusted local host.
- Add external gateway profile.
- Add hosted gateway profile.
- Store profile settings locally.

Depends on: P3.1, P2.3.

Output:

- Connection profile UI and storage.
- Profile tests.

#### P3.3 Implement local daemon discovery

- Probe default loopback gateway.
- Validate `/healthz` and `/api/v1/capabilities`.
- Show daemon status.
- Allow manual base URL override.

Depends on: P3.2, P1.2.

Output:

- Discovery command.
- Discovery UI.

#### P3.4 Implement optional daemon supervision

- Add native command to start daemon when configured.
- Track daemon process ownership.
- Stop only daemon processes owned by the desktop app.
- Surface logs and startup errors.

Depends on: P3.3.

Output:

- Local supervisor.
- Process ownership tests.

#### P3.5 Configure Tauri capabilities

- Restrict frontend commands.
- Scope file/folder selection permissions.
- Scope notification permissions.
- Scope local process supervision commands.
- Separate main window privileges from any future auxiliary windows.

Depends on: P3.1, P0.5.

Output:

- Capability files.
- Security review checklist.

#### P3.6 Implement native settings and keychain hooks

- Store non-secret settings locally.
- Store sensitive local credentials in OS keychain where needed.
- Add redaction helpers.
- Add credential status UI.

Depends on: P3.2, P3.5.

Output:

- Settings service.
- Keychain integration.

#### P3.7 Implement desktop-native actions

- Open repository folder.
- Reveal workspace folder where allowed.
- Copy path or issue link.
- Open external Linear issue link.
- Send native notification on run completion or approval request.

Depends on: P3.5, P2 views.

Output:

- Native action menu.
- Notification tests.

#### P3.8 Optimize Tauri stream path

- Connect frontend streams to the selected local host profile.
- Use in-process Rust channels when the host is embedded or directly attached.
- Use native local IPC when the host is a separate local process and benchmarks justify it.
- Use Tauri channels from the Rust backend to the webview where benchmarks show benefit.
- Use zero-copy-friendly Rust frame buffers internally where practical, with copies only at trust, process, or webview boundaries.
- Keep HTTP/WebSocket fallback.
- Ensure local and remote transports expose the same frontend contract.

Depends on: P2.9, P1.8, P0.4.

Output:

- Desktop local transport adapter.
- Stream benchmark update.

### Exit criteria

- Desktop app connects to local OpenSymphony and renders live state.
- Desktop connection profiles cover local native/local gateway and hosted remote modes.
- Desktop app can optionally supervise a local daemon safely.
- Tauri capabilities are explicit.
- Terminal/log rendering remains responsive in local mode.

## 7. Phase P4: Linear task graph management

### Objective

Provide a rich OpenSymphony UI for navigating and managing Linear projects, milestones, issues, and sub-issues.

### Dependencies

P1.4, P1.9, P2.7, P3 optional for desktop packaging.

### Tasks

#### P4.1 Expand Linear read coverage

- Ensure GraphQL queries cover projects.
- Ensure project milestones are available.
- Ensure issues and sub-issues are available.
- Ensure relations, labels, priorities, statuses, assignees, estimates, comments, and attachments are available where needed.
- Add schema drift validation.

Depends on: P1.4.

Output:

- Linear query coverage matrix.
- GraphQL schema drift tests.

#### P4.2 Implement task graph cache

- Cache Linear entities with sync timestamps.
- Cache OpenSymphony runtime overlays.
- Join cache records into task graph DTOs.
- Add invalidation and refresh behavior.

Depends on: P4.1.

Output:

- Task graph cache.
- Sync tests.

#### P4.3 Implement milestone mutation API

- Create milestone as Linear project milestone.
- Update milestone fields.
- Return action receipt.
- Publish task graph update event.

Depends on: P1.9, P4.1.

Output:

- Milestone mutation endpoints.
- Mutation tests with fake Linear.

#### P4.4 Implement issue mutation API

- Create issue as Linear issue.
- Assign project and milestone.
- Update title, description, status, priority, labels, assignee, and estimate.
- Add relation support.

Depends on: P4.3.

Output:

- Issue mutation endpoints.
- Mutation tests.

#### P4.5 Implement sub-issue mutation API

- Create sub-issue as Linear sub-issue.
- Update sub-issue fields.
- Add relation/blocker support.
- Add comments and evidence notes.

Depends on: P4.4.

Output:

- Sub-issue mutation endpoints.
- Mutation tests.

#### P4.6 Build task graph editor UI

- Add editable milestone/issue/sub-issue views.
- Add create dialogs.
- Add inline field editing where safe.
- Add dependency editor.
- Add comments/evidence editor.
- Add optimistic UI only after server acknowledgement rules are clear.

Depends on: P4.3, P4.4, P4.5, P2.7.

Output:

- Task graph editor.
- UI tests.

#### P4.7 Add runtime overlay interaction

- Link issue and sub-issue rows to active or last run.
- Show failed, blocked, queued, running, complete, and stale states.
- Show workspace, harness, diff summary, validation, and retry badges.
- Add filters for runtime states.

Depends on: P4.2, P2.8.

Output:

- Runtime overlay UI.
- Filter tests.

### Exit criteria

- User can browse Linear project structure inside OpenSymphony.
- User can create and update milestones, issues, and sub-issues through the gateway.
- Runtime state is visible directly on task graph nodes.

## 8. Phase P5: OpenHands rich runtime integration

### Objective

Turn OpenHands runtime data into a rich, inspectable, reconnect-safe client experience.

### Dependencies

P1.5, P1.7, P1.8, P2.8, P2.9, P0.6.

### Tasks

#### P5.1 Normalize OpenHands event types

- Add typed handling for high-value OpenHands events.
- Preserve unknown events.
- Map events to OpenSymphony run lifecycle updates.
- Map events to terminal/log streams where applicable.
- Add raw event inspector support.

Depends on: P1.7, P0.6.

Output:

- Event normalization module.
- Contract tests.

#### P5.2 Implement runtime state mirror

- Maintain active conversation/session state.
- Track execution status.
- Track readiness and reconnect status.
- Track last known event and history sync status.

Depends on: P5.1.

Output:

- Runtime state mirror.
- State tests.

#### P5.3 Implement rich event timeline

- Group related runtime events.
- Summarize tool calls.
- Link events to files, commands, logs, terminal panes, and diffs.
- Show raw event details on demand.

Depends on: P5.1, P2.8.

Output:

- Timeline UI.
- Event grouping tests.

#### P5.4 Implement terminal/log stream association

- Associate output with run, command, issue, and sub-issue.
- Expose scrollback reads.
- Expose live stream frames.
- Add search and jump-to-event.

Depends on: P2.9, P1.8.

Output:

- Terminal/log service endpoints.
- Frontend terminal/log integration.

#### P5.5 Implement diff and validation evidence views

- Show changed files.
- Show per-file diffs.
- Show validation commands and results.
- Link evidence to completion state and Linear comments.

Depends on: P1.6, P2.8.

Output:

- Diff viewer.
- Validation evidence UI.

#### P5.6 Implement approval center v1

- Normalize approval requests from orchestrator/harness where supported.
- Add pending approval list.
- Add approve, deny, and explain actions.
- Add audit trail.

Depends on: P1.9, P1.10, P2.8.

Output:

- Approval center.
- Approval action tests.

#### P5.7 Add run action bar

- Retry.
- Cancel where supported.
- Rehydrate.
- Add comment.
- Create follow-up issue or sub-issue.
- Open workspace or debug view where allowed.

Depends on: P1.10, P4.5, P2.8.

Output:

- Run action bar.
- Action UI tests.

### Exit criteria

- Active OpenHands runs are visible as structured timelines, streams, diffs, and evidence.
- Reconnect does not lose committed runtime events.
- Users can take supported actions from the run detail view.

## 9. Phase P6: Collaborative planning and kickoff flow

### Objective

Expand existing OpenSymphony planning skills into a rich planning workspace that adapts GSD-2's task-creation workflow and creates Linear milestones, issues, and sub-issues after review.

### Dependencies

P1.9, P4.3, P4.4, P4.5, P2.4, P2.5.

### Tasks

#### P6.1 Define planning artifact schema

- Define intake artifact.
- Define project context artifact.
- Define requirements artifact.
- Define research brief artifact.
- Define codebase analysis artifact.
- Define architecture notes artifact.
- Define risk register artifact.
- Define milestone plan artifact.
- Define issue plan artifact.
- Define sub-issue plan artifact.
- Define dependency graph artifact.
- Define verification plan artifact.
- Define plan validation artifact.
- Define Linear draft artifact.
- Define publish receipt artifact.

Depends on: P0.3.

Output:

- Planning artifact schema.
- Artifact validation tests.

#### P6.2 Implement planning session service

- Create planning session.
- Store conversation turns.
- Store artifact revisions.
- Track session state.
- Support artifact diffs.
- Support review comments.
- Persist planning sessions and artifacts.
- Render markdown or structured projections for review, prompt context, audit history, and diffs.

Depends on: P6.1, P1.1.

Output:

- Planning session APIs.
- Session persistence tests.

#### P6.3 Integrate repository, Linear, and research analysis

- Analyze repository structure.
- Read existing Linear project/task graph.
- Summarize existing constraints.
- Research public documentation, APIs, ecosystem references, and other relevant external sources.
- Summarize codebase conventions, ownership boundaries, risks, and likely integration points.
- Attach analysis to planning session.

Depends on: P6.2, P4.2.

Output:

- Analysis artifact generator.
- Analysis tests with fixture repos.

#### P6.4 Wrap `create-implementation-plan` as a generator stage

- Adapt the existing skill into a planning-stage generator.
- Feed repository and project analysis into the generator.
- Store generated output as structured artifacts.
- Preserve Linear-native milestone/issue/sub-issue terminology in generated artifacts.
- Generate milestone-level goals, issue-level vertical deliverables, sub-issue-level execution units, acceptance criteria, and verification expectations.
- Generate initial dependency relationships.
- Allow selective regeneration.

Depends on: P6.2, P6.3.

Output:

- Implementation plan generator integration.
- Artifact tests.

#### P6.5 Build milestone/issue/sub-issue compiler

- Convert implementation plan artifacts into Linear milestone/issue/sub-issue hierarchy.
- Enforce taxonomy: milestone equals Linear project milestone, issue equals Linear issue, sub-issue equals Linear sub-issue.
- Treat GSD-2 milestone or phase-level planning as milestone-level planning.
- Treat GSD-2 slices as Linear issues and GSD-2 tasks as Linear sub-issues.
- Require acceptance criteria for issues.
- Require verification expectations for sub-issues.
- Require dependencies where applicable.
- Flag underspecified sub-issues.

Depends on: P6.4.

Output:

- Plan compiler.
- Sub-issue and dependency checks.

#### P6.6 Build dependency graph and plan checks

- Generate dependency graph edges across milestones, issues, and sub-issues.
- Detect cycles and missing blockers.
- Identify parallelizable work.
- Flag unclear scope, missing acceptance criteria, and missing verification expectations.
- Store graph and check results as planning artifacts.

Depends on: P6.2, P6.5.

Output:

- Dependency graph generator.
- Plan quality tests.

#### P6.7 Build planning workspace UI

- Add conversation pane.
- Add artifact pane.
- Add hierarchy editor.
- Add dependency editor.
- Add acceptance criteria editor.
- Add verification editor.
- Add research and codebase analysis panes.
- Add dependency graph view.
- Add plan validation UI.
- Add diff view between artifact revisions.

Depends on: P6.2, P6.5, P6.6, P2.5.

Output:

- Planning workspace.
- UI tests.

#### P6.8 Build Linear draft preview

- Generate draft GraphQL mutation payloads.
- Show created/updated milestones, issues, sub-issues, comments, and relations.
- Show warnings and missing fields.
- Require explicit approval before publish.

Depends on: P6.5, P4 mutation APIs.

Output:

- Linear draft preview UI.
- Draft validation tests.

#### P6.9 Wrap `convert-tasks-to-linear`

- Adapt existing skill into a publish stage.
- Prefer gateway-mediated mutations for UI flow.
- Preserve compatibility with checked-in Linear GraphQL helper/query assets.
- Store publish receipts.

Depends on: P6.8, P4.3, P4.4, P4.5.

Output:

- Publish flow.
- Publish receipt tests.

#### P6.10 Add plan validation and publish readiness checks

- Verify all sub-issues have acceptance criteria or a parent issue acceptance criterion.
- Verify verification commands or validation expectations are present where needed.
- Verify dependencies do not form invalid cycles.
- Verify required research and codebase analysis artifacts exist.
- Verify issue and milestone success criteria exist before publish.
- Verify Linear entities were created or updated.

Depends on: P6.9.

Output:

- Publish readiness report.
- Linear publish integration.

### Exit criteria

- User can collaboratively generate, edit, review, and publish Linear milestones, issues, and sub-issues.
- The planning flow demonstrates the adapted GSD-2 task-creation workflow: guided interview, research, codebase analysis, milestone/issue/sub-issue decomposition, dependency graph review, and Linear draft publishing.
- Published items appear in the task graph with runtime readiness state.
- Existing planning skills are integrated into a persistent UI workflow.

## 10. Phase P7: Web client and external gateway mode

### Objective

Deploy the shared frontend as a browser app that connects to an OpenSymphony Gateway.

### Dependencies

P2, P1.8, P3 is helpful but not required.

### Tasks

#### P7.1 Create web app entrypoint

- Add browser app wrapper.
- Configure environment-based gateway URL.
- Configure static asset build.
- Ensure no Tauri API dependency leaks into browser bundle.

Depends on: P2.1, P2.3.

Output:

- Web app build.
- Browser smoke tests.

#### P7.2 Implement browser transport

- Use HTTP for reads and mutations.
- Use WebSocket or SSE for event streams based on gateway capabilities.
- Use binary WebSocket for terminal/log streams where enabled.
- Evaluate JSON-RPC 2.0 over WebSocket for hosted bidirectional control and subscriptions.
- Require cursor replay, idempotency keys, action receipts, and monotonic event sequences for any selected remote transport.
- Support reconnect and cursor replay.

Depends on: P2.3, P1.8.

Output:

- Browser transport adapter.
- Reconnect tests.

#### P7.3 Add web deployment with gateway

- Serve static assets from the OpenSymphony Gateway.
- Support base path deployment.
- Support cache-busted assets.
- Support local development proxy.

Depends on: P7.1.

Output:

- Gateway-served web client.
- Deployment docs.

#### P7.4 Add separately deployed web mode

- Allow frontend to be deployed independently.
- Configure gateway base URL.
- Handle CORS and auth preparation.
- Verify WebSocket origins and security configuration.

Depends on: P7.2.

Output:

- Separate web deployment option.
- CORS/origin test plan.

#### P7.5 Add hosted-auth placeholders

- Add login state UI.
- Add unauthorized and forbidden states.
- Add organization/project selection placeholders.
- Keep local unauthenticated mode simple.

Depends on: P7.1.

Output:

- Auth-aware UI shell.
- Placeholder tests.

### Exit criteria

- Browser app can connect to local or external gateway.
- Browser app renders the same core project, task graph, run, stream, and planning views.
- Static app can be served by gateway or deployed separately.

## 11. Phase P8: Hosted mode alpha

### Objective

Add server-hosted multi-user execution where clients can disconnect and work continues on the server.

### Dependencies

P1 stable gateway, P7 web client, P4 task graph, P5 OpenHands runtime integration.

### Tasks

#### P8.1 Add identity and tenant model

- Add users.
- Add organizations.
- Add memberships.
- Add roles.
- Add project access rules.
- Add tenant-scoped entity IDs.

Depends on: P1.1.

Output:

- Hosted identity schema.
- RBAC tests.

#### P8.2 Add authentication

- Select auth provider strategy.
- Add login/logout/session endpoints.
- Add API, WebSocket, and JSON-RPC-over-WebSocket auth middleware if JSON-RPC is selected.
- Add local development auth bypass mode only for trusted dev.

Depends on: P8.1, P7.5.

Output:

- Auth middleware.
- Web client login flow.

#### P8.3 Add hosted secrets storage

- Add encrypted secret references.
- Store Linear credentials.
- Store provider credentials.
- Store harness credentials.
- Add rotation and revocation paths.
- Add redaction tests.

Depends on: P8.1, P8.2.

Output:

- Secret store integration.
- Secret redaction tests.

#### P8.4 Add hosted Linear connection

- Support organization or project Linear connections.
- Support API-key mode initially or OAuth if selected.
- Scope access by tenant and project.
- Add sync worker configuration.

Depends on: P8.3, P4.1.

Output:

- Hosted Linear connection service.
- Tenant isolation tests.

#### P8.5 Add hosted workspace isolation

- Select isolation layer: containers, VMs, or managed sandbox.
- Define workspace lifecycle.
- Define network policy.
- Define filesystem policy.
- Define cleanup and retention.
- Integrate workspace manager with logical workspace IDs.

Depends on: P0.9, P8.1.

Output:

- Workspace isolation implementation.
- Isolation test plan.

#### P8.6 Add hosted OpenHands runtime pool

- Run OpenHands agent-server instances under platform control.
- Route conversations to isolated workspaces.
- Manage health checks and capacity.
- Attach runtime event streams server-side.
- Add resource limits.

Depends on: P8.5, P5.

Output:

- Hosted harness runtime manager.
- Runtime pool tests.

#### P8.7 Persist runs independent of clients

- Ensure run lifecycle persists without active streams.
- Ensure event journal continues recording.
- Ensure workspace cleanup follows policy.
- Ensure clients reconnect after long disconnects.

Depends on: P1.7, P8.6.

Output:

- Client disconnect tests.
- Long-running run tests.

#### P8.8 Add hosted audit and metrics

- Audit login/logout.
- Audit project access changes.
- Audit secret changes.
- Audit action mutations.
- Track resource usage.
- Track stream and run metrics.

Depends on: P8.2, P8.3, P1.9.

Output:

- Audit log.
- Metrics dashboard foundation.

#### P8.9 Add hosted admin controls

- View users and projects.
- View active runs.
- View runtime capacity.
- Cancel or kill runs within permission boundaries.
- Rotate or revoke credentials.
- Configure quotas.

Depends on: P8.8.

Output:

- Admin UI alpha.
- Admin action tests.

### Exit criteria

- Multiple users can connect to hosted OpenSymphony.
- Runs continue after clients disconnect.
- User permissions protect project and run access.
- Hosted workspaces are isolated.
- Web and desktop clients can connect to hosted gateway profiles.

## 12. Phase P9: Future harness, auth, and model readiness

### Objective

Prepare and then optionally implement subscription credentials, API-compatible model configuration metadata, Codex app-server, and cross-harness routing.

This phase is intentionally separate from the initial rich-client and hosted alpha work. Subscription credentials are independent from Codex app-server: OpenAI ChatGPT/Codex can be implemented first for the existing OpenHands agent-server harness, while Codex app-server can later reuse the same model and credential settings.

### Dependencies

P0.7, P0.8, P1 harness interfaces, P5 OpenHands normalization, P8 secrets for hosted credential storage if hosted auth is implemented.

### Tasks

#### P9.1 Finalize harness adapter interface

- Confirm OpenHands adapter fits the interface.
- Confirm Codex app-server can fit the interface.
- Confirm future Pi/Rust-native adapter can fit the interface.
- Add harness capability matrix.

Depends on: P5, P0.7.

Output:

- Stable `HarnessAdapter` interface.
- Harness capability DTO.

#### P9.2 Implement model and credential settings

- Preserve API-compatible settings for `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`.
- Add subscription-backed credential settings with OpenAI ChatGPT/Codex as the first provider adapter.
- Add credential status endpoint.
- Add local keychain or isolated OpenHands auth-directory storage for desktop/local mode.
- Add hosted secret storage or credential-broker integration where needed.
- Represent which harnesses can consume each model setting.

Depends on: P0.8, P3.6, P8.3 if hosted.

Output:

- Model and credential settings model.
- Credential status UI.

#### P9.3 Integrate first subscription adapter for OpenHands

- Support OpenHands SDK `LLM.subscription_login(vendor="openai", ...)`.
- Support browser login flow where the SDK/client supports it.
- Support device-code flow where supported.
- Store credentials through the selected storage provider.
- Construct a subscription-backed `LLM` for OpenHands `Agent` and `Conversation` creation.
- Expose login status, account identity where available, auth mode, and expiration state.
- Avoid undocumented auth implementation.

Depends on: P9.2, P5.

Output:

- Feature-gated OpenHands subscription credential adapter.
- Auth integration tests.

#### P9.4 Implement model configuration metadata

- Add model configuration profiles based on base URL, model string, credential reference, and harness capability.
- Add optional operator-supplied metadata for context window, reasoning effort, cost profile, and recommended task types.
- Add model configuration UI.
- Support dynamic routing from configured model settings and harness capabilities.
- Preserve arbitrary configured model strings for API-compatible OpenHands usage.

Depends on: P9.2, P9.3.

Output:

- Model configuration service.
- Model configuration UI.

#### P9.5 Implement Codex app-server local stdio prototype

- Launch `codex app-server` over stdio.
- Initialize JSON-RPC session.
- Start thread and turn.
- Read notifications.
- Normalize basic events.
- Add schema generation to CI or dev tooling.
- Reuse model and credential settings where Codex credentials or model selection are needed.

Depends on: P9.1.

Output:

- Feature-gated Codex local prototype.
- Contract tests.

#### P9.6 Benchmark Codex WebSocket mode

- Start Codex app-server with loopback WebSocket.
- Configure auth flags where supported.
- Measure throughput and queue behavior.
- Measure reconnect behavior.
- Verify non-loopback exposure is blocked unless secured.

Depends on: P9.5.

Output:

- Codex WebSocket benchmark report.
- Production readiness recommendation.

#### P9.7 Implement Codex approval mapping

- Map Codex approval requests to OpenSymphony approval center.
- Send approval decisions back to Codex.
- Audit all decisions.

Depends on: P9.5, P5.6.

Output:

- Codex approval bridge.
- Approval contract tests.

#### P9.8 Implement cross-harness routing policy alpha

- Define routing rules by task type, model profile, harness capability, cost, speed, and user policy.
- Add explicit user override.
- Add route decision audit events.
- Add dry-run route preview.

Depends on: P9.1, P9.4, P9.5 if Codex routing is enabled.

Output:

- Routing policy engine alpha.
- Route decision tests.

### Exit criteria

- Subscription credentials can be used as a feature-gated credential path for OpenHands agent-server, starting with OpenAI ChatGPT/Codex.
- Codex app-server can be used as a feature-gated harness in local prototype mode.
- Subscription credentials, API-compatible model configuration, and harness adapters are separated.
- API-compatible model settings flow through the existing OpenHands configuration path by setting `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`.
- Cross-harness routing has a clear policy model.

## 13. Phase P10: Hardening, tests, performance, and release quality

### Objective

Ensure the system is reliable, secure, fast, and maintainable across local desktop, web, external server, hosted mode, and future integration paths.

### Dependencies

Runs throughout P1 through P9.

### Tasks

#### P10.1 Contract test suite

- Gateway schema tests.
- Event replay tests.
- Stream reconnect tests.
- Linear fake server tests.
- OpenHands fake server tests.
- Codex fake server tests when enabled.
- Auth and RBAC tests.

Depends on: P1 onward.

Output:

- Contract test suite.

#### P10.2 End-to-end local tests

- Start local OpenSymphony.
- Connect desktop client.
- Load dashboard.
- Load task graph.
- Start or observe a run.
- Render event timeline.
- Render logs and diffs.
- Retry or rehydrate a run.

Depends on: P3, P5.

Output:

- Local E2E test suite.

#### P10.3 Web E2E tests

- Load web client.
- Connect to gateway.
- Simulate reconnect.
- Verify task graph and run views.
- Verify planning flow draft.

Depends on: P7.

Output:

- Web E2E suite.

#### P10.4 Hosted E2E tests

- User login.
- Project access.
- Linear sync.
- Hosted run continues after disconnect.
- Second user observes permitted state.
- Unauthorized user blocked.
- Workspace cleanup.

Depends on: P8.

Output:

- Hosted E2E suite.

#### P10.5 Performance gates

- Dashboard snapshot latency gate.
- Event stream latency gate.
- Terminal/log throughput gate.
- UI frame responsiveness gate.
- Large project task graph load gate.
- Planning artifact render gate.

Depends on: P2, P3, P5, P7.

Output:

- Performance benchmark suite.
- CI performance report.

#### P10.6 Security review

- Tauri capability review.
- Hosted auth review.
- Secret redaction tests.
- Workspace isolation review.
- WebSocket origin and auth review.
- Audit log completeness review.
- Dependency vulnerability review.

Depends on: P3, P8, P9.

Output:

- Security review report.
- Remediation list.

#### P10.7 Accessibility review

- Keyboard navigation.
- Focus states.
- Screen-reader labels.
- Color-independent status.
- Terminal/log copy and search.
- Reduced-motion behavior if needed.

Depends on: P2 UI completion.

Output:

- Accessibility checklist.
- Remediation list.

#### P10.8 Documentation and developer experience

- Local development setup.
- Desktop build instructions.
- Web deployment instructions.
- Gateway API docs.
- Harness adapter docs.
- Linear GraphQL mutation docs.
- Planning flow docs.
- Hosted deployment docs.
- Troubleshooting and diagnostics docs.

Depends on: All major phases.

Output:

- Documentation set.
- Developer onboarding checklist.

## 14. Implementation task dependency table

| Task | Depends on | Blocks |
|---|---|---|
| P0.1 Inventory current control plane | None | P1.1 |
| P0.2 Domain vocabulary and IDs | None | P1.4, P6.5 |
| P0.3 Gateway schemas | P0.1, P0.2 | P1, P2.2 |
| P0.4 Stream benchmarks | None | P1.8, P2.9, P3.8 |
| P0.5 Tauri validation | None | P3.5 |
| P0.6 OpenHands validation | None | P5 |
| P0.7 Codex spike | None | P9 |
| P0.8 Subscription credentials and model configuration spike | None | P9.2, P9.3, P9.4 |
| P0.9 Hosted threat model | None | P8.5 |
| P1 Gateway foundation | P0 | P2, P4, P5, P7, P8 |
| P2 Shared frontend | P1 reads and streams | P3, P4 UI, P5 UI, P6 UI, P7 |
| P3 Desktop client | P2, P0.5 | Local rich-client release |
| P4 Task graph management | P1, P2 | P6, hosted project management |
| P5 OpenHands runtime views | P1, P2, P0.6 | Rich run operations, P8 runtime |
| P6 Planning flow | P4, P2 | Spec-driven project kickoff release |
| P7 Web client | P1, P2 | P8 hosted UX |
| P8 Hosted mode | P1, P4, P5, P7 | Hosted release, hosted credentials |
| P9 Future harness/auth | P0.7, P0.8, P1, P5 | Subscription credentials, model configuration, and Codex roadmap |
| P10 Hardening | All phases | Release quality |

## 15. Initial milestone proposal

### Milestone M1: Gateway and stream contract

Includes P0 and P1.

Success criteria:

- `/api/v1` exists.
- Snapshot, task graph, run detail, event history, and live stream contracts work.
- Initial action receipt framework works.
- Existing OpenSymphony local run behavior remains compatible.

### Milestone M2: Shared client and desktop alpha

Includes P2 and P3.

Success criteria:

- Desktop app connects to local OpenSymphony through the best available local profile: native/in-process or IPC where supported, loopback gateway fallback otherwise.
- Desktop app can also connect to a remote hosted gateway profile.
- Dashboard, task graph, and run detail views work.
- Terminal/log prototype handles representative output.
- Local daemon discovery and optional supervision work.

### Milestone M3: Task graph operations and OpenHands run UI

Includes P4 and P5.

Success criteria:

- Linear milestones, issues, and sub-issues are navigable and editable.
- OpenHands runtime data renders as structured events, logs, diffs, and validation evidence.
- Supported run actions are available through the UI.

### Milestone M4: Collaborative planning alpha

Includes P6.

Success criteria:

- User can create a planning session.
- User can generate and edit a Linear milestone/issue/sub-issue hierarchy.
- GSD-2-inspired planning checks for scope clarity, research coverage, codebase analysis, dependencies, acceptance criteria, and verification expectations are visible.
- User can preview and publish Linear mutations after approval.
- Published items appear in task graph and can be scheduled.

### Milestone M5: Web client and external gateway

Includes P7.

Success criteria:

- Browser client works against gateway.
- Web app can be served by gateway or deployed separately.
- Core UI behavior matches desktop for remote connections.

### Milestone M6: Hosted alpha

Includes P8.

Success criteria:

- Authenticated users access hosted projects.
- Server-owned runs continue after clients disconnect.
- Hosted workspaces are isolated.
- Web and desktop clients can connect to hosted gateway.

### Milestone M7: Future provider, harness, and model integration readiness

Includes P9 discovery-to-alpha work.

Success criteria:

- Subscription credentials work as a feature-gated credential path for OpenHands agent-server, starting with OpenAI ChatGPT/Codex.
- Codex app-server prototype is feature-gated.
- Model and credential settings preserve API-compatible `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY` configuration.
- Model configuration metadata supports routing by configured base URL, model string, and harness capability.
- Cross-harness routing policy is defined.

## 16. Recommended first implementation cut

The first development cut should be intentionally narrow:

1. Add `/api/v1/capabilities`.
2. Stabilize `/api/v1/dashboard/snapshot`.
3. Add `/api/v1/projects/{id}/taskgraph` for read-only task graph data.
4. Add `/api/v1/runs/{id}` and `/api/v1/runs/{id}/events`.
5. Add replayable `/api/v1/events?cursor=`.
6. Build shared frontend skeleton.
7. Build Tauri desktop wrapper.
8. Render dashboard, task graph, and run detail from live gateway data.
9. Add terminal/log pane prototype.
10. Add stream reconnect and stale-data handling.

This cut produces a useful rich-client alpha while validating the gateway and stream contracts that every later feature depends on.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-407 contributed: PR #125: feat(api-client): browser transport streaming, replay, and remote protocols (COE-407) (merge `4d70347`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-407: Browser Transport And Remote Stream Protocols

## Source refs

- COE-407

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
