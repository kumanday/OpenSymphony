# Project Milestones

This milestone index covers the new rich client, hosted mode, collaborative planning, subscription-auth, and future harness work defined in:

- `PRODUCT.md`
- `docs/hosted-client-PRD.md`
- `docs/host-client-architecture.md`
- `docs/host-client-implementation_plan.md`
- `docs/okf-memory-spec.md`
- `docs/llm-wiki-graph-view-spec.md`
- `docs/opensymphony-acp-debugging-spec.md`

## M6: Gateway And Stream Contract

Goal: Establish the versioned OpenSymphony Gateway, public DTOs, replayable event streams, action receipts, and feasibility baselines for desktop, web, hosted, and high-throughput transports.

Tasks:

- OSYM-700 Current Gateway Inventory And Vocabulary
- OSYM-701 Gateway Schemas And Stream Feasibility
- OSYM-702 Gateway Module, Capabilities, And Dashboard Snapshot
- OSYM-703 Task Graph, Run Detail, File, And Diff Read APIs
- OSYM-704 Event Journal And Stream Broker
- OSYM-705 Action Receipts And Initial Run Actions

## M7: Shared Client And Desktop Alpha

Goal: Build the shared TypeScript client foundation and Tauri desktop shell that can connect to local and hosted OpenSymphony profiles through a common frontend contract.

Tasks:

- OSYM-710 Frontend Workspace And Shared Schemas
- OSYM-711 Gateway API Client, Transport Adapters, And Reducers
- OSYM-712 App Shell, Dashboard, Task Graph, And Run Views
- OSYM-713 Terminal And Log Renderer Prototype
- OSYM-714 Tauri Shell And Security Capabilities
- OSYM-715 Desktop Connection Profiles And Daemon Management
- OSYM-716 Desktop Settings, Keychain, And Native Actions
- OSYM-717 Desktop Local Stream Optimization
- OSYM-718 Desktop Alpha Recovery

## M8: Task Graph Operations And OpenHands Run UI

Goal: Provide Linear-native task graph operations and a rich OpenHands runtime interface with timelines, streams, diffs, validation evidence, approvals, and run actions.

Tasks:

- OSYM-720 Linear Read Coverage And Task Graph Cache
- OSYM-721 Linear Milestone, Issue, And Sub-Issue Mutations
- OSYM-722 Task Graph Editor And Runtime Overlay UI
- OSYM-723 OpenHands Event Normalization And Runtime Mirror
- OSYM-724 Runtime Timeline And Terminal/Log Association
- OSYM-725 Diff, Validation, Approval, And Run Action Views

## M9: Collaborative Planning Alpha

Goal: Implement the adapted GSD-2 task-creation workflow as a reviewable OpenSymphony planning workspace that produces Linear milestones, issues, sub-issues, dependencies, acceptance criteria, verification expectations, and publish payloads.

Tasks:

- OSYM-730 Planning Artifact Schema And Session Service
- OSYM-731 Repository, Linear, And Research Analysis
- OSYM-732 Implementation Plan Generator Stage
- OSYM-733 Milestone, Issue, And Sub-Issue Compiler
- OSYM-734 Dependency Graph And Plan Checks
- OSYM-735 Planning Workspace UI
- OSYM-736 Linear Draft Preview And Publish Flow

## M9.5: Developer Build Acceleration

Goal: Reduce OpenSymphony-on-OpenSymphony development compile cost after the planning alpha while preserving bundled, turnkey DuckDB behavior for normal users and releases.

Tasks:

- OSYM-737 DuckDB Prebuilt Developer Build Mode
- OSYM-738 Non-Interactive Init For Automation

## M10: Web Client And External Gateway

Goal: Deploy the shared frontend as a browser app that connects to local, external, and hosted gateways with reconnect-safe remote transport behavior.

Tasks:

- OSYM-740 Web App Entry And Deployment Modes
- OSYM-741 Browser Transport And Remote Stream Protocols
- OSYM-742 Hosted Auth Placeholders And Web Parity

## M10.3: Codex And Subscription Readiness

Goal: Deliver local Codex app-server support and ChatGPT subscription credential foundations before full hosted mode.

Placement: After M10 web client/external gateway work and before M10.5 OKF Memory Bundle Foundation, M11 Hosted Alpha, and the broader M12 provider/harness backlog.

Tasks:

- OSYM-760 Harness Adapter And Capability Model
- OSYM-761 Model And Credential Settings
- OSYM-762 OpenHands Subscription Credential Adapter
- OSYM-763 Model Configuration UI And Routing Metadata
- OSYM-764 Codex App-Server Prototype And Benchmarks
- OSYM-766 ChatGPT OAuth For Codex Harness
- OSYM-767 Codex Production Harness Enablement
- OSYM-765 Codex Approvals And Cross-Harness Routing

## M10.5: OKF Memory Bundle Foundation

Goal: Evolve project memory into OKF-conformant bundles while keeping existing memory query, docs sync, and privacy behavior intact.

Tasks:

- OSYM-800 OKF Bundle Schema And Legacy Capsule Mapping
- OSYM-801 OKF Writer, Lint, And Migration Fixtures
- OSYM-802 Catalog Reindex And Query Compatibility From OKF
- OSYM-803 OKF Export, Import, And Visibility Boundaries
- OSYM-804 Docs Sync And MCP Admin Parity For OKF

## M11: Hosted Alpha

Goal: Add hosted multi-user execution where server-owned runs continue after clients disconnect and permissions, secrets, workspaces, audit, and administration are enforced centrally.

Tasks:

- OSYM-750 Hosted Identity, Auth, And RBAC
- OSYM-751 Hosted Secrets And Linear Connections
- OSYM-752 Hosted Workspace Isolation And Runtime Pool
- OSYM-753 Client-Independent Run Persistence
- OSYM-754 Hosted Audit, Metrics, And Admin Controls
- OSYM-755 Hosted Subscription Credential Broker And Secret Store

## M11.5: LLM Wiki Graph View

Goal: Add a shared web and desktop OKF memory graph view with gateway DTOs, graph extraction, community detection, an accessible inspector, and live privacy-aware memory integration.

Tasks:

- OSYM-820 Memory Graph DTOs And Gateway Endpoints
- OSYM-821 Graph Extraction, Metrics, And Community Pipeline
- OSYM-822 Shared Graph Frontend Package And Reducers
- OSYM-823 Three.js Graph Renderer And Worker Layouts
- OSYM-824 Concept Inspector, Search, Filters, And Accessibility Fallback
- OSYM-825 Live Memory Graph Integration And Privacy Gates
- OSYM-826 Graph Scale, Visual Regression, And Web/Desktop Hardening

## M12: Provider, Harness, And Model Readiness

Goal: Hold follow-on provider, harness, and model readiness work after the near-term Codex subscription-readiness milestone.

Tasks:

- Follow-on tasks to be assigned after M10.3 Codex And Subscription Readiness.

## M12.5: ACP Debugging And IDE Attach

Goal: Expose OpenSymphony issue debug sessions through Zed ACP while preserving issue workspace, OpenHands conversation, and debug stream ownership in OpenSymphony.

Tasks:

- OSYM-840 Debug Attachment Core Refactor
- OSYM-841 ACP Stdio Server Protocol Adapter
- OSYM-842 Zed Static Agent Configuration And Setup UX
- OSYM-843 Tauri Debug-In-Zed Launch Action
- OSYM-844 Default Debug UX Transition And CLI Compatibility
- OSYM-845 ACP Debug Integration Tests And Failure Guidance

## M13: Hardening And Release Quality

Goal: Prove the system through contract, end-to-end, performance, security, accessibility, and documentation work.

Tasks:

- OSYM-770 Contract And Local End-To-End Tests
- OSYM-771 Web, Hosted, And Performance Tests
- OSYM-772 Security, Accessibility, Documentation, And Developer Experience
