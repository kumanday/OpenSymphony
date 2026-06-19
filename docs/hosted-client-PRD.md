# OpenSymphony Rich Client, Hosted Mode, and Collaborative Planning PRD

## 1. Product objectives

OpenSymphony will evolve into an integrated orchestration platform for AI-assisted software delivery. The new development work will add a rich desktop and web client, a stronger OpenSymphony Gateway, Linear-native task management, hosted execution mode, and a collaborative planning flow that converts project intent into executable Linear-backed work.

The product objectives are:

1. Provide a high-throughput rich client experience for observing and controlling OpenSymphony runs.
2. Create a first-class task graph UI for Linear projects, milestones, issues, and sub-issues joined with OpenSymphony runtime state.
3. Preserve the orchestrator as the source of truth for scheduling, run state, workspaces, retries, and reconciliation.
4. Support local desktop use first while designing the host-client architecture for hosted multi-user execution.
5. Add a web client that shares most frontend code with the desktop client.
6. Expand existing planning skills into an interactive human-AI project kickoff flow that creates Linear milestones, issues, and sub-issues.
7. Treat OpenAI ChatGPT/Codex subscription auth as a provider capability that can serve the existing OpenHands agent-server path and future harnesses, while keeping Codex app-server itself as a separate future integration.
8. Maintain cross-harness, cross-provider, and cross-model flexibility as a core orchestration design goal.

## 2. Definitions

### OpenSymphony Gateway

The versioned server API exposed by the OpenSymphony runtime. It provides snapshots, streams, details, task graph data, mutations, action dispatch, authentication hooks, and hosted-mode boundaries.

### Client

A desktop or web user interface that connects to the OpenSymphony Gateway. The client is never the scheduling source of truth. It renders state, streams live updates, collects human intent, and sends versioned actions to the gateway.

### Desktop client

A Tauri application with a webview frontend, a Rust native shell, native menus, native settings, keychain support, optional local daemon supervision, and high-throughput local stream support.

### Web client

A browser-deployed version of the same frontend. It connects to the gateway through HTTP and WebSocket APIs and does not assume local filesystem or OS-level privileges.

### Harness

An agent execution substrate that can run or control coding-agent sessions. The initial harness is OpenHands agent-server. Future harnesses include Codex app-server and other local or remote agent runtimes.

### Provider

A model transport source configured through OpenHands-compatible settings such as `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`.

### Model

A model string configured through `LLM_MODEL` or produced by a subscription-backed `LLM` construction path.

### Project

A Linear project and repository execution scope managed by OpenSymphony.

### Milestone

A Linear project milestone. A milestone is a major checkpoint or delivery stage inside a project.

### Issue

A Linear issue under a milestone. An issue is a demoable vertical capability or deliverable unit.

### Sub-Issue

A Linear sub-issue under an issue. A sub-issue is a bounded unit of implementation, validation, documentation, or cleanup that can be assigned to a human or executed by an agent run.

### GSD-2 Prior Art

GSD-2 provides a reference workflow for guided project kickoff: interview the user, clarify vision and scope, research relevant public information, analyze the existing codebase, synthesize a plan, decompose the work, and review the dependency graph. Its milestone or phase-level planning maps to Linear milestones, its slices map to Linear issues, and its tasks map to Linear sub-issues.

### Run

A specific OpenSymphony attempt to execute a Linear issue or sub-issue through a harness, including workspace, conversation/session, events, logs, terminal streams, diffs, validation, outcome, and retry metadata.

## 3. Users and use cases

### Individual developer

- Connect a repository and Linear project.
- Run OpenSymphony locally.
- Use the desktop client to inspect active and historical runs.
- Use the planning flow to turn a feature idea into Linear milestones, issues, and sub-issues.
- Launch, retry, cancel, rehydrate, and debug agent work.

### Technical lead

- Navigate a project plan by milestone, issue, and sub-issue.
- Inspect which work is running, blocked, queued, retried, or complete.
- Review agent-generated plan drafts before they create or update Linear entities.
- Inspect diffs, validation evidence, and failure reasons before approving follow-up actions.
- Compare runtime outcomes to acceptance criteria.

### Team member

- Access hosted OpenSymphony through a browser without installing local tooling.
- Review project progress and agent activity.
- Participate in planning discussions and approval gates.
- Reconnect to long-running work after disconnecting.

### Platform administrator

- Configure hosted OpenSymphony deployment.
- Manage users, tenants, repositories, Linear connections, secrets, harness pools, quotas, and audit logs.
- Ensure hosted workspaces are isolated and observable.

## 4. Functional requirements

### 4.1 OpenSymphony Gateway

#### 4.1.1 Versioned API namespace

The gateway must expose a versioned API namespace, initially `/api/v1`, for all client-facing endpoints.

Required endpoint groups:

- Health and readiness.
- Projects and repositories.
- Task graph data.
- Runs and workspaces.
- Runtime events.
- Terminal and log streams.
- Files, changed files, and diffs.
- Approvals and human actions.
- Planning sessions and planning artifacts.
- Settings and capability discovery.
- Auth and identity hooks for hosted mode.

#### 4.1.2 Snapshot and detail APIs

The gateway must provide efficient initial snapshots and detail reads.

Required reads:

- Global dashboard snapshot.
- Project list.
- Project detail.
- Linear task graph for a project.
- Milestone, issue, and sub-issue detail.
- Run list by sub-issue, issue, milestone, project, and status.
- Run detail.
- Workspace detail.
- Conversation or harness session summary.
- Event history with cursor support.
- Changed files and per-file diffs.
- Validation command history and result summaries.
- Approval requests and decisions.

#### 4.1.3 Streaming APIs

The gateway must provide replayable streams for live state.

Required stream behavior:

- Support reconnect with a cursor.
- Preserve event ordering by sequence number.
- Deduplicate repeated events.
- Support bounded queues and backpressure.
- Allow clients to resume after temporary disconnects.
- Expose stale-data and degraded-stream state to clients.
- Separate control events from high-volume terminal/log frames when needed.

Initial stream types:

- Dashboard events.
- Project/task graph events.
- Run lifecycle events.
- Harness runtime events.
- Terminal output or terminal cell delta frames.
- Log frames.
- Approval request events.
- Planning-session events.

#### 4.1.4 Mutations and actions

The gateway must expose action endpoints that accept user intent rather than private internal mutations.

Initial action categories:

- Start or dispatch eligible work.
- Retry failed work.
- Cancel queued or running work when supported.
- Pause or resume a run when supported.
- Rehydrate a broken or intentionally reset conversation.
- Add a comment or evidence note to an issue.
- Transition issue status when permitted.
- Create a follow-up issue or sub-issue.
- Create or update milestones, issues, and sub-issues through Linear GraphQL.
- Approve, deny, or explain an approval request.
- Open or focus a run, workspace, terminal, or diff view.

Every mutation must be idempotent where practical and must return a correlation ID that appears in the event stream.

#### 4.1.5 Capability discovery

The gateway must expose capabilities so clients can adapt to local, external local, and hosted deployments.

Capability payloads must include:

- Supported gateway API version.
- Enabled tracker adapters.
- Enabled harness adapters.
- Supported actions by entity and state.
- Terminal/log streaming mode.
- Planning flow availability.
- Hosted-mode auth and user capability state.
- Experimental and unavailable integrations.
- Model configuration metadata when implemented.

### 4.2 Desktop client

#### 4.2.1 Tauri shell

The desktop app must use Tauri with:

- A shared web frontend rendered in the Tauri webview.
- A Rust shell for native integrations.
- Native settings storage.
- OS keychain support for local secrets where needed.
- Optional local OpenSymphony daemon discovery and supervision.
- Native file/folder selection for local repositories.
- System notifications for long-running work.
- Keyboard shortcuts and command palette integration.
- Tauri capabilities configured so frontend privileges are explicit and window-scoped.

#### 4.2.2 Local and hosted connection modes

The desktop client must support connecting to:

- A local OpenSymphony daemon started separately.
- A local OpenSymphony daemon supervised by the desktop app when configured.
- A local OpenSymphony host embedded in or directly attached to the Tauri Rust shell if that packaging model is selected.
- An external local OpenSymphony gateway on loopback or a trusted local network.
- A hosted OpenSymphony server.

The desktop app and the web app must render the same product state, but they do not need the same physical transport. For local desktop operation, the Rust shell should use the highest-throughput safe path available: in-process channels when the host is embedded, native IPC such as Unix domain sockets or named pipes when the host is external but local, Tauri channels from Rust to the webview, and loopback HTTP/WebSocket as the compatibility fallback. For hosted operation, the desktop app should behave like the web app and connect through authenticated HTTPS/WSS gateway transports.

#### 4.2.3 High-throughput rendering

The desktop client must provide a responsive interface for high-volume runtime streams.

Requirements:

- Render terminal/log streams without blocking primary UI interactions.
- Use worker-based rendering for terminal panes where feasible.
- Use compact frames for high-volume streams.
- Coalesce bursty updates without losing authoritative history.
- Maintain a stable scrollback model.
- Allow jump-to-latest and scrollback inspection.
- Display terminal, logs, events, diffs, and task state in linked panes.

#### 4.2.4 Native UX

The desktop app must offer the cleanest and fastest experience:

- Fast startup and reconnect.
- Persistent window layout.
- Resizable panes and tabs.
- Project switcher.
- Command palette.
- Keyboard-first navigation.
- Local notifications.
- Native credential prompts where appropriate.
- Optional tray status for running local orchestrator state.

### 4.3 Web client

The web client must share the majority of frontend code with the desktop client.

Requirements:

- Same task graph, dashboard, planning, run detail, event timeline, diff, and approval components.
- Browser transport adapter using HTTP and WebSocket.
- Hosted authentication flow.
- No dependency on Tauri APIs.
- Deployable as static assets served by the OpenSymphony Gateway.
- Deployable separately and configurable with a gateway base URL.
- Safe behavior under browser refresh and reconnect.

### 4.4 Shared frontend application

The shared frontend must include:

- Dashboard.
- Project navigator.
- Task graph explorer.
- Milestone detail.
- Issue detail.
- Sub-issue detail.
- Run detail.
- Terminal/log pane.
- Diff viewer.
- Event timeline.
- Approval center.
- Planning workspace.
- Settings and capability pages.
- User/session panel for hosted mode.
- Transport abstraction for desktop-local, desktop-remote, and web-remote connections.

The frontend state model must be reducer-driven and stream-aware. It must support initial snapshot, live updates, replay, reconnect, and stale-data states.

### 4.5 Linear task graph

#### 4.5.1 Data model

The task graph must join Linear data with OpenSymphony runtime overlays.

Linear fields:

- Project.
- Project milestones.
- Issues.
- Sub-issues.
- Status.
- Assignee.
- Team.
- Priority.
- Labels.
- Estimate.
- Due dates.
- Relations.
- Comments.
- Attachments.
- Project updates.

OpenSymphony runtime overlays:

- Eligibility state.
- Queue state.
- Active run state.
- Last run outcome.
- Retry count.
- Workspace path or logical workspace ID.
- Harness type.
- Conversation/session ID.
- Last event timestamp.
- Diff summary.
- Validation status.
- Cost and token summary when available.
- Blockers and dependency status.

#### 4.5.2 Views

Required views:

- Project overview.
- Milestone board.
- Issue list.
- Sub-issue list.
- Dependency graph.
- Runtime status board.
- Failed or blocked work view.
- Recently changed work view.
- Agent activity view.

#### 4.5.3 Mutations

The UI must support creating and updating Linear-backed project structure through gateway-mediated GraphQL operations.

Required mutations:

- Create milestone as Linear project milestone.
- Update milestone name, description, target date, and ordering metadata when available.
- Create issue under project and milestone.
- Update issue title, description, status, priority, labels, assignee, estimate, and relations.
- Create sub-issue under issue.
- Update sub-issue fields.
- Add comments and evidence notes.
- Attach PR, branch, diff summary, or validation artifacts where supported.
- Create blocking or related issue relationships.

The gateway must validate user permissions and preserve an audit trail for hosted mode.

### 4.6 Collaborative planning flow

#### 4.6.1 Goals

The planning flow must help a human and AI collaborator turn a project idea into an executable OpenSymphony plan and Linear task graph.

The flow must produce:

- Project summary.
- Goals and non-goals.
- Requirements.
- Assumptions.
- Risks.
- Architecture notes.
- Milestone definitions.
- Issue definitions.
- Sub-issue definitions.
- Acceptance criteria.
- Verification commands.
- Dependencies.
- Research findings.
- Codebase analysis.
- Dependency graph.
- Plan validation findings.
- Linear draft payload.

#### 4.6.2 User experience

The planning workspace must include:

- Conversational pane for human-AI discussion.
- Artifact pane for structured outputs.
- Repository analysis pane.
- Plan hierarchy editor.
- Dependency editor.
- Verification and acceptance criteria editor.
- Linear draft preview.
- Diff view between plan revisions.
- Dependency graph view.
- Plan validation panel.
- Approval gate before creating or updating Linear entities.

#### 4.6.3 Flow stages

Required stages:

1. Intake: collect product goal, repository, tracker project, constraints, and success criteria.
2. Discovery: inspect repository structure and existing Linear state.
3. Requirements: extract functional and nonfunctional requirements into a reviewable contract.
4. Research: inspect public documentation, APIs, ecosystem references, and other relevant external sources.
5. Codebase analysis: identify implementation boundaries, conventions, risks, and likely integration points.
6. Architecture: summarize implementation strategy and technical risks.
7. Milestone planning: define Linear milestones with goals, constraints, risks, dependencies, and success criteria.
8. Issue planning: define demoable vertical deliverables per milestone, with acceptance criteria and dependency notes.
9. Sub-issue planning: define bounded execution units per issue.
10. Dependency graph: identify ordering, blockers, parallelizable work, and cross-issue relationships.
11. Verification planning: define test, lint, build, review, and evidence expectations.
12. Review: allow human edits, plan diffs, selective regeneration, and explicit approval.
13. Publish: create or update Linear project milestones, issues, sub-issues, comments, and relations.

#### 4.6.4 Existing skill integration

The planning flow must leverage and extend current OpenSymphony skills:

- `create-implementation-plan` should become one artifact-producing stage inside the interactive planning workflow.
- `convert-tasks-to-linear` should become the publish stage that creates or updates Linear entities after review.
- Linear GraphQL helper/query assets should remain the canonical query and mutation surface for agent-visible Linear operations unless replaced by a versioned gateway service with equivalent schema coverage.
- GSD-2-inspired checks should run before publish: missing acceptance criteria, missing verification expectations, invalid dependency cycles, unclear scope, and underspecified sub-issues.

### 4.7 OpenHands harness integration

OpenHands agent-server remains the initial harness integration.

Requirements:

- OpenSymphony server, not the frontend, owns the OpenHands REST and WebSocket connection.
- REST operations handle creation, message sending, run triggering, event search, recovery, and reconciliation.
- WebSocket streams provide live runtime events.
- Gateway normalizes OpenHands events into OpenSymphony events.
- Unknown OpenHands event types must be preserved as raw payloads and must not crash the runtime.
- Initial REST event sync, WebSocket attach, readiness barrier, and post-ready reconciliation must be preserved.
- Event ordering and deduplication must happen server-side.
- UI displays normalized events, plus raw payload inspection for debugging.
- When subscription auth is enabled, OpenSymphony can use the documented OpenHands SDK subscription-login path to construct a subscription-backed `LLM` for the OpenHands `Agent` and `Conversation`.
- OpenHands subscription auth must be modeled as subscription credential configuration.

### 4.8 Future Codex app-server integration

Codex app-server support is future scope, but the initial architecture must include the required seams.

Future requirements:

- Add `CodexHarnessAdapter` under the same harness abstraction as OpenHands.
- Support JSON-RPC method calls and notifications.
- Use stdio transport as the default local integration path.
- Treat WebSocket transport as experimental until proven stable and secured.
- Generate TypeScript and JSON Schema artifacts from the installed Codex app-server version where possible.
- Normalize Codex thread, turn, approval, message, and tool events into OpenSymphony run events.
- Map Codex approvals into the OpenSymphony approval center.
- Map Codex conversation/thread history into run detail views.
- Support model selection through Codex configuration or app-server request parameters where supported.
- Reuse model configuration and credential settings instead of introducing Codex-owned credential storage.
- Benchmark throughput, queue behavior, reconnect behavior, and approval latency before enabling production use.

### 4.9 OpenHands model configuration and subscription integration

OpenHands model configuration uses `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`. This API-compatible configuration path remains the default model settings path. Subscription-backed login support adds a credential path that can construct an OpenHands `LLM` for `openhands agent-server`. OpenAI ChatGPT/Codex login is the first subscription implementation. Future subscription providers and future Codex app-server support can consume the same settings and credential concepts.

Requirements:

- Support subscription-backed sign-in through documented OpenHands SDK or provider client flows, starting with OpenAI ChatGPT/Codex.
- Preserve API-compatible configuration through `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`.
- Support browser OAuth and device-code flows where the SDK/client supports them.
- Store local credentials in an OS credential store, keychain, or isolated OpenHands auth directory in desktop/local mode.
- Store hosted credentials as encrypted user-scoped or organization-scoped secrets, with refresh tokens kept out of harness workspaces.
- Expose configured base URL, model string, credential status, subscription account identity where available, and credential expiry state.
- Expose harness capability and model configuration metadata for user configuration and routing.
- Use documented authentication flows.
- Keep subscription-backed credentials distinct from API-compatible `LLM_API_KEY` settings.

### 4.10 Hosted mode

Hosted mode is a follow-on release, but all data models and APIs should be designed for it.

Hosted requirements:

- User authentication.
- Organization and tenant model.
- Role-based access control.
- Repository connection management.
- Linear OAuth or secure API-key connections.
- Per-user and per-organization secrets.
- Server-owned orchestrator processes.
- Server-owned harness runtime pools.
- Persistent workspaces isolated by tenant, project, and run.
- Runs continue after all clients disconnect.
- WebSocket reconnect and event replay.
- Central logs, metrics, and audit trails.
- Quotas and resource limits.
- Admin visibility and kill controls.
- Workspace cleanup and retention policies.
- Deployment support for web client served by gateway or deployed separately.

### 4.11 Security and permissions

Required security properties:

- UI privileges must be explicitly scoped.
- Tauri capabilities must limit frontend access to native commands.
- Server mutations must check user identity, tenant, role, project access, and action capability.
- Secrets must never appear in logs, events, terminal summaries, or issue comments.
- Hosted workspaces must isolate users and tenants.
- External harness endpoints must require explicit configuration and secure transport in hosted mode.
- Audit logs must record user actions and sensitive configuration changes.
- Approval prompts must clearly describe the actor, target, command, file, issue, run, and risk where known.
- Client-side state must never be the final authority for permissions.

### 4.12 Observability

Required observability:

- Structured gateway logs.
- Run-level event journal.
- Harness event summaries.
- Stream connection metrics.
- Queue depth and backpressure metrics.
- Per-project and per-run latency metrics.
- Retry and failure counters.
- Validation command history.
- Cost and token metrics where available.
- Hosted audit events.
- Diagnostics bundle export for support and debugging.

## 5. Nonfunctional requirements

### 5.1 Performance

- Dashboard snapshot should load quickly for typical projects.
- Event stream latency should be low enough to feel live during active runs.
- Terminal/log rendering should remain responsive during bursts.
- The UI must coalesce updates when rendering cannot keep up.
- The server must maintain authoritative event history even when clients drop frames.
- Large projects must support pagination, filtering, and incremental detail loading.

### 5.2 Reliability

- Client reconnect must recover from the last known cursor.
- Server restart recovery must preserve run and workspace state where supported.
- Unknown harness events must be stored and surfaced without failing the run.
- Linear GraphQL failures must fail clearly and must not corrupt OpenSymphony state.
- UI crashes or disconnects must not stop orchestrator execution.

### 5.3 Portability

- Desktop client must support macOS, Linux, and Windows where Tauri supports the target platform.
- Web client must support modern Chromium, Firefox, and Safari versions where feasible.
- Hosted server must be deployable as a service with configurable storage, secrets, and workspace isolation.

### 5.4 Accessibility

- Keyboard navigation must be first-class.
- All core views must provide readable text alternatives to visual-only state.
- Color must not be the only status signal.
- Pane focus and command shortcuts must be visible.
- Terminal and log panes must support copy, search, and stable scrollback.

### 5.5 Maintainability

- Gateway contracts must be versioned.
- Harness adapters must have contract tests.
- Linear GraphQL operations must have schema drift checks.
- Shared frontend components must be transport-agnostic.
- Desktop-only and web-only code must be isolated behind adapters.
- Experimental integrations must be feature-gated.

## 6. Release structure

### Release 1: Local rich client foundation

Required:

- Gateway API expansion.
- Shared frontend foundation.
- Tauri desktop shell.
- Dashboard.
- Task graph read views.
- Run detail read views.
- OpenHands runtime event visualization through OpenSymphony.
- Basic action intents: retry, cancel where supported, rehydrate, comment, open debug view.
- Capability discovery.
- Initial terminal/log stream rendering.

### Release 2: Linear task management and planning alpha

Required:

- Linear-backed milestone, issue, and sub-issue creation/update.
- Plan artifact model.
- Conversational planning workspace.
- Milestone/issue/sub-issue editor.
- Linear draft preview.
- Review and publish flow.
- Dependency and verification fields.
- Plan validation and publish approval flow.

### Release 3: Web client and external server mode

Required:

- Web client deployment.
- Browser transport adapter.
- Shared authentication placeholder.
- External OpenSymphony Gateway connection.
- Reconnect and replay parity with desktop.
- Static frontend deployable with gateway or separately.

### Release 4: Hosted alpha

Required:

- Auth and user model.
- Tenant and organization model.
- Hosted workspace isolation.
- Server-owned orchestrator and harness pools.
- Persistent runs independent of clients.
- Central secrets.
- Central logs, metrics, and audit trail.
- Web client login.
- Desktop hosted connection profile.

### Release 5: Future harness and model integrations

Candidate features:

- Subscription credential support for OpenHands agent-server and future harnesses, starting with OpenAI ChatGPT/Codex.
- API-compatible model configuration UI for `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`.
- Codex app-server harness adapter.
- Model configuration metadata for dynamic routing.
- Cross-harness routing policies.
- Harness capability benchmarks.
- Pi or other Rust-native harness integration where justified.

## 7. Acceptance criteria

### Gateway

- A client can fetch a project snapshot, task graph, run detail, and event history through versioned APIs.
- A client can subscribe to live run events, disconnect, reconnect with a cursor, and see no missing committed events.
- Mutating actions return correlation IDs and appear in the event stream.
- Unknown harness events are preserved and visible in diagnostics.

### Desktop client

- A user can connect to a local OpenSymphony daemon and monitor active work.
- A user can connect the same desktop app to a hosted OpenSymphony gateway and monitor remote work.
- A local desktop connection can use a native high-throughput transport when available and fall back to loopback HTTP/WebSocket.
- A user can navigate from project to milestone to issue to sub-issue to run to file diff.
- A user can inspect live logs or terminal output while the UI remains responsive.
- A user can retry or rehydrate a failed run from the UI when the action is available.
- The app stores local connection profiles and credentials securely where relevant.

### Web client

- A user can open the web client, authenticate in hosted mode, and view the same project/task/run state as desktop.
- A user can reconnect after page refresh without losing committed event history.
- The web client can be served by the gateway or deployed independently.

### Linear task graph

- A user can see Linear project milestones, issues, and sub-issues using Linear-native names.
- A user can create a milestone, issue, and sub-issue through the OpenSymphony UI.
- Runtime overlays show active run, last outcome, workspace, diff summary, validation status, and retry state.

### Planning flow

- A user can start with a project prompt and produce a reviewed plan containing Linear milestones, issues, sub-issues, acceptance criteria, dependencies, and verification commands.
- The planning flow demonstrates the adapted GSD-2 kickoff workflow: guided interview, research, codebase analysis, milestone/issue/sub-issue decomposition, dependency graph review, and Linear draft publishing.
- A user can edit the generated plan before publishing.
- Publishing creates or updates Linear entities only after explicit approval.
- Published work is ready for OpenSymphony scheduling.

### Hosted mode

- A user can disconnect all clients while a run continues on the server.
- A second user with permission can view the same project and run state.
- Tenant and project permissions prevent unauthorized access.
- Hosted logs and events support audit and diagnostics.

### Future integration readiness

- The codebase has harness adapter interfaces that can support OpenHands and Codex app-server without changing task graph or run UI contracts.
- The codebase has credential settings that can support subscription auth separately from API-compatible `LLM_API_KEY` configuration and from harness adapter implementations.
- Model routing can use configured base URL, model string, and harness capabilities.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-419 contributed: PR #126: Load desktop task graph dependencies from Linear (merge `64242a6`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-419: Hosted Auth Placeholders And Web Parity

## Source refs

- COE-419

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
