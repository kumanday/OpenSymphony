# OpenSymphony Product Vision

## Product thesis

OpenSymphony turns AI-assisted software development from a collection of disconnected chats, terminals, issue trackers, and local scripts into an integrated execution system for software projects. It coordinates project intent, task planning, issue tracking, agent execution, workspace isolation, live runtime telemetry, human review, and delivery evidence through one orchestrated control plane.

The product is built around a simple operating principle: the orchestrator owns the project execution state, while user interfaces and agent harnesses connect to that state through versioned contracts. Humans can plan, inspect, intervene, and approve work through rich desktop and web clients. Agents can execute work inside isolated workspaces through supported harnesses. Linear remains the first task-tracking system of record, while OpenSymphony becomes the execution layer that connects Linear work items to concrete agent runs, artifacts, logs, diffs, and completion evidence.

OpenSymphony should feel like a project operating room for AI software delivery. A user can create or connect a project, break down the work into Linear milestones, issues, and sub-issues, launch agent execution, watch progress in real time, inspect terminals and diffs, guide stuck runs, and convert successful work back into issue updates, comments, PRs, and milestone progress.

## Current foundation

OpenSymphony already provides a strong execution foundation:

- A Rust orchestrator that polls Linear for active issues and schedules eligible work.
- Hierarchy-aware scheduling so parent issues can wait for sub-issues before advancing.
- Deterministic per-issue workspaces with lifecycle hooks.
- OpenHands agent-server integration for remote agent execution through HTTP operations and WebSocket runtime events.
- Retry, reconciliation, cleanup, and restart recovery behavior.
- A control plane API that exposes health, snapshots, and runtime events.
- A FrankenTUI operator interface for monitoring local runs.
- GraphQL-only Linear helper/query assets for agent-side Linear reads and writes.
- Existing project skills, including `create-implementation-plan` and `convert-tasks-to-linear`, that can be expanded into a richer planning and kickoff workflow.

The next product step is to evolve this from a local-first orchestrator with a terminal UI into a richer OpenSymphony platform with shared desktop and web clients, a stronger host-client boundary, a native task graph interface, high-throughput runtime visualization, hosted execution, and future-ready cross-harness and cross-model support.

## Product pillars

### 1. Orchestrated execution

OpenSymphony should remain an orchestration system, not just a frontend for an agent. The orchestrator is responsible for:

- Determining which work is eligible to run.
- Enforcing concurrency limits and dependency constraints.
- Creating and tracking per-issue workspaces.
- Starting, monitoring, retrying, pausing, cancelling, and reconciling agent runs.
- Persisting runtime state and event history.
- Mapping agent outcomes back to project and issue state.
- Preserving correctness even when a UI disconnects, crashes, or is never opened.

This creates a durable distinction between the execution system and the interface. A client may present a rich experience, but correctness belongs to the orchestrator.

### 2. Rich desktop and web clients

The current terminal UI is valuable for local monitoring, but the product opportunity is much larger. OpenSymphony should add a rich client experience that includes:

- A project dashboard showing health, active runs, blocked work, retries, costs, and recent events.
- A task graph view for projects, milestones, issues, sub-issues, dependencies, ownership, and runtime state.
- A run detail view with live events, terminal streams, logs, diffs, files changed, approvals, and validation evidence.
- A planning and kickoff workspace for collaborative human-AI project analysis and task decomposition.
- A command palette and contextual action system for dispatching, retrying, pausing, cancelling, rehydrating, commenting, transitioning issues, and creating follow-up work.
- A shared visual language across desktop and web.

The desktop client should be the premium local experience: faster, cleaner, lower-latency, and more deeply integrated with the operating system. The web client should provide broad access with no installation requirement and should be deployable either with the hosted OpenSymphony server or as a separate frontend pointed at an OpenSymphony Gateway.

### 3. High-throughput terminal, harness, and app integration

The core advantage over ordinary agent harness frontends is a higher-throughput integration between project state, runtime state, terminal state, and human control.

The client should support high-volume streams without turning the interface into slow text polling. Runtime events, terminal deltas, logs, diffs, and approval requests should flow through bounded, replayable streams with backpressure and reconnect behavior. The app should present these streams as a coherent operating interface:

- The user sees what the agent is doing now.
- The user sees which project, milestone, issue, sub-issue, workspace, branch, file, run, and terminal the activity belongs to.
- The user can jump from an event to the relevant file, diff, log, issue, approval, or terminal.
- The agent run remains attached to the orchestrator, not to the life of a local window.

### 4. Linear-native task management with OpenSymphony runtime overlays

Linear should remain the initial external task system of record. OpenSymphony should provide its own interface for navigating and managing Linear-backed projects, milestones, issues, and sub-issues.

The product should treat Linear data and OpenSymphony runtime data as a joined task graph:

- Linear project: the product or delivery initiative.
- Linear project milestone: a major delivery stage or checkpoint.
- Linear issue: a demoable vertical capability or deliverable unit.
- Linear sub-issue: a bounded implementation, validation, documentation, or cleanup unit small enough for one agent run or one bounded sequence of runs.
- Linear comments, relations, attachments, statuses, and project updates: the external collaboration and audit layer.
- OpenSymphony workspaces, runs, harness sessions, event journals, diffs, validation commands, costs, retries, and completion evidence: the execution overlay.

This interface should be more deeply integrated than opening Linear in a tab. The user should navigate project structure, execution state, and work evidence in one place.

### 5. Cross-harness, cross-provider, cross-model orchestration

OpenSymphony should support a flexible orchestration layer across harnesses and model configurations. OpenHands agent-server remains the initial execution substrate. Future harness support should include `codex app-server` and, where appropriate, Rust-native or in-process harnesses such as `pi_agent_rust`.

The orchestrator should not assume that one harness or one model fits all work. It should eventually route work by:

- Harness capability: agent-server runtime, JSON-RPC app-server, local in-process SDK, sandbox support, browser support, file editing model, approval model, and event contract.
- API-compatible model configuration: `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`.
- Model capability: coding strength, speed, context window, cost, reasoning effort, and tool support.
- Task shape: planning, implementation, refactor, debugging, testing, validation, documentation, browser-based verification, or code review.

The current OpenHands path already supports API-compatible model configuration through `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`. Subscription authentication should be added as an additional credential path that can support multiple subscription providers over time. OpenAI ChatGPT/Codex OAuth is the first concrete subscription integration because OpenHands SDK can use subscription login to construct an `LLM` for the existing OpenHands agent-server path. The architecture should include the settings seams that let subscription credentials serve OpenHands now and future harnesses later.

### 6. Subscription-backed access as a harness-orthogonal capability

OpenSymphony should support subscription authentication as a credential capability that can be used by multiple harnesses. The first implementation is OpenAI ChatGPT/Codex subscription authentication. For OpenHands, the documented OpenHands SDK subscription-login path performs OAuth, stores refreshable credentials, and constructs a normal subscription-backed `LLM` object that can be attached to an `Agent` and `Conversation` using `openhands agent-server`. Future subscription providers and future `codex app-server` support can reuse the same credential and model-selection concepts without owning the auth abstraction.

The direction is:

- Support subscription-backed sign-in through documented SDK or provider client flows, starting with OpenAI ChatGPT/Codex.
- Preserve API-compatible model configuration through `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`.
- Expose configured model, base URL, credential status, and subscription account status through settings.
- Store local credentials through OS keychain facilities or an isolated OpenHands auth directory in desktop/local mode.
- Store server-side credentials through encrypted per-user or per-organization secret storage in hosted mode, with refresh tokens kept out of agent workspaces.
- The implementation should use documented SDK/client behavior and respect OpenAI account and workspace policy boundaries.

Configured endpoints expose different model sets, so the base URL and model string remain the basis for user configuration and dynamic model selection. The reasoning effort should be configurable or dynamically selected based on the task complexity.

### 7. Hosted mode

Hosted mode is a first-class product direction. In hosted mode, the heavy execution system runs on a server or server cluster instead of the user's computer. Users connect through desktop or web clients.

Hosted mode enables:

- Multiple users sharing one managed OpenSymphony server.
- No local setup for non-operator users.
- Centralized Linear, repository, and model/harness configuration.
- Long-running agent work that continues while users disconnect or shut down their computers.
- Shared project visibility for teams.
- Central logs, metrics, audit trails, secrets, quotas, and policy enforcement.
- Better workspace isolation through containers, VMs, or managed sandboxes.
- Web access for users who do not install a desktop app.

Hosted mode should not change the core orchestration contract. It changes deployment, isolation, authentication, resource management, and availability expectations.

### 8. GSD-2-inspired collaborative project kickoff

OpenSymphony should expand its existing planning skills into a full collaborative flow for project analysis, specification, scaffolding, and kickoff. GSD-2 provides a strong reference for a guided task-creation workflow that starts with user interview and ends with structured implementation work.

The user-facing planning taxonomy is the Linear taxonomy:

- Project: a Linear project and repository scope.
- Milestone: a Linear project milestone, representing a major delivery stage or checkpoint.
- Issue: a Linear issue under a milestone, representing one demoable vertical capability or deliverable.
- Sub-issue: a Linear sub-issue under an issue, representing a bounded implementation, validation, documentation, or cleanup unit.

GSD-2 concepts map into Linear as follows:

- GSD-2 milestone or phase-level planning maps to a Linear project milestone.
- GSD-2 slice maps to a Linear issue.
- GSD-2 task maps to a Linear sub-issue.

The kickoff workflow should adapt these GSD-2 patterns:

- Guided interview to clarify project vision, success criteria, scope, constraints, and open questions.
- Research pass for public documentation, APIs, ecosystem norms, and relevant external references.
- Repository analysis pass for existing architecture, code ownership boundaries, risks, conventions, and likely integration points.
- Planning synthesis that turns interview, research, and codebase analysis into a coherent milestone plan.
- Milestone, issue, and sub-issue decomposition with clear dependency relationships.
- Acceptance criteria and validation expectations attached to the right level of the hierarchy.
- Human review and editing before the generated plan creates or updates Linear entities.

The flow should guide a human and AI collaborator through:

1. Project intake and goal clarification.
2. Repository and system analysis.
3. Requirements extraction and risk identification.
4. Architecture and implementation strategy.
5. Milestone definition.
6. Issue decomposition into demoable vertical deliverables.
7. Sub-issue decomposition into bounded execution units.
8. Acceptance criteria and verification commands.
9. Dependency and sequencing analysis.
10. Human review, plan diffs, and explicit approval.
11. Linear project/milestone/issue/sub-issue creation.

This extends `create-implementation-plan` and `convert-tasks-to-linear` from command-like skills into a rich UX with persistent planning artifacts, editable drafts, diffs between plan versions, dependency graph validation, and direct creation of Linear-backed project structure.

## Value proposition

### For individual developers

OpenSymphony reduces the friction between planning and execution. A developer can turn a feature idea into structured work, launch agents against real tasks, inspect outputs, and recover from failures without juggling terminals, chats, issue pages, and ad hoc notes.

### For technical leads

OpenSymphony provides a live execution layer for project plans. A lead can create milestones, issues, and sub-issues, assign or route work to agents, monitor progress, inspect evidence, and intervene when work is blocked. The system provides visibility into what agents did, where they did it, and why the result is ready or not ready.

### For teams

Hosted mode enables shared AI execution infrastructure. A team can standardize workflows, model access, secrets, repository configuration, audit trails, and workspace isolation while allowing users to access the system through a browser or desktop app.

### For organizations

OpenSymphony offers an architecture for controlled adoption of AI coding agents. It centralizes policy and observability while preserving flexibility across harnesses, providers, and models. It can integrate with existing issue-tracking and repository practices rather than replacing them.

## Scope

### Initial rich-client scope

- Versioned OpenSymphony Gateway APIs for project/task-graph/run snapshots, event streams, task graph data, and selected mutations.
- Shared TypeScript frontend foundation for desktop and web.
- Tauri desktop shell with a Rust backend for native integration and local high-throughput streams.
- OpenHands agent-server runtime visualization through OpenSymphony, not direct client-to-OpenHands attachment.
- Linear-backed project, milestone, issue, and sub-issue navigation.
- Rich run detail views with events, logs, terminal output, diffs, approvals, and validation evidence.
- Local mode support with a path toward hosted mode.

### Near-term expansion scope

- First version of the collaborative project kickoff and planning flow.
- Linear GraphQL mutations for creating and updating milestones, issues, and sub-issues.
- Draft, review, approve, and publish workflow for generated plans.
- More complete action surface: retry, cancel, pause, resume, rehydrate, comment, transition issue, create follow-up work, and attach evidence.
- Web client deployment behind the OpenSymphony Gateway.

### Hosted mode scope

- Server-side authentication and authorization.
- Multi-user and multi-tenant data model.
- Persistent runs independent of client connection state.
- Centralized secrets and model/provider configuration.
- Container, VM, or managed sandbox isolation for hosted workspaces.
- Centralized event journal, metrics, logs, and audit trails.
- Web client served either by the hosted server or by a separately deployed static frontend.

### Future-ready scope

- Subscription-backed sign-in support for the existing OpenHands agent-server path and future harnesses, starting with OpenAI ChatGPT/Codex.
- Codex app-server as an optional harness in addition to OpenHands agent-server.
- Model configuration UI for API-compatible endpoints and subscription-backed OpenHands login.
- Cross-harness routing and benchmarking.
- Additional task trackers beyond Linear through a tracker adapter contract.

## Non-goals

- Replacing Linear as the initial project system of record.
- Reimplementing the complete OpenHands or Codex agent loop inside OpenSymphony.
- Allowing client UI state to become authoritative for orchestration correctness.
- Depending on direct browser access to private harness streams when the orchestrator should own runtime attachment and reconciliation.
- Shipping hosted multi-tenant execution before the local and gateway contracts are stable.

## Product principles

1. The orchestrator is authoritative.
2. The UI observes, explains, and issues versioned intents.
3. Every agent run belongs to a Linear issue or sub-issue, workspace, harness session, event journal, and evidence trail.
4. High-throughput streams need replay, backpressure, and bounded memory.
5. Linear data and runtime data should be joined into one task graph experience.
6. Hosted mode should change deployment and isolation, not the core orchestration model.
7. Harness support should be adapter-based, typed, versioned, and benchmarked.
8. Model selection should be provider-aware and capability-aware.
9. Planning artifacts should be reviewable before they mutate Linear.
10. Humans should always be able to understand what happened, what is running, what is blocked, and what can be done next.

## Success measures

- Time from repository bootstrap to first visible orchestrated run.
- Time from project idea to reviewed Linear milestone/issue/sub-issue structure.
- Percentage of active runs visible with live status, evidence, and reconnect-safe history.
- Runtime event latency from orchestrator to client.
- Terminal/log stream throughput without UI stalls.
- Correct replay after client reconnect.
- Reduction in manually created Linear planning work.
- Reduction in abandoned or unrecoverable agent runs.
- Number of concurrent active execution units supported locally and in hosted mode.
- User trust signals: clear state, clear evidence, clear intervention options, and predictable recovery.

## Source constraints

- OpenSymphony is a Rust orchestration implementation that connects to Linear and uses OpenHands agent-server as its current runtime substrate.
- OpenHands agent-server provides an HTTP/WebSocket service for remote agent execution, workspace isolation, container orchestration, centralized management, and multi-user systems.
- OpenHands SDK subscription login can use OpenAI/ChatGPT OAuth to construct a subscription-backed `LLM` that works with the existing OpenHands agent-server flow.
- Codex app-server uses JSON-RPC over stdio by default and has an experimental WebSocket transport; it is designed for rich Codex clients with authentication, conversation history, approvals, and streamed events.
- API-compatible OpenHands model access is configured with `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY`.
- Linear exposes a public GraphQL API and supports projects, project milestones, issues, sub-issues, comments, relations, attachments, and OAuth/API-key authentication patterns.
- Tauri provides a Rust backend plus webview frontend model; high-throughput client streams should prefer channels or purpose-built binary streams rather than generic JSON event fanout.
- GSD-2 research source: https://github.com/gsd-build/gsd-2 and its public docs as reviewed on 2026-05-10.
- GSD-2 demonstrates a useful guided planning workflow with project interview, requirements clarification, research, codebase analysis, milestone planning, slice decomposition, task decomposition, and dependency mapping. OpenSymphony should adapt those task-creation concepts and expose them through Linear milestone/issue/sub-issue terminology.
