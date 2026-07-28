# Project Milestones

This milestone index covers rich client, hosted mode, collaborative planning,
subscription-auth, future harness, and multi-repository orchestration work
defined in:

- `PRODUCT.md`
- `docs/hosted-client-PRD.md`
- `docs/host-client-architecture.md`
- `docs/host-client-implementation_plan.md`
- `docs/specs/okf-memory-spec.md`
- `docs/specs/llm-wiki-graph-view-spec.md`
- `docs/specs/opensymphony-acp-debugging-spec.md`
- `docs/specs/desktop-run-detail-operations-spec.md`
- `docs/specs/tui-dependency-gutter-spec.md`
- `docs/specs/opensymphony_tree_sitter_ast_spec.md`
- `docs/specs/desktop-app-installer-auto-update-spec.md`
- `docs/specs/codex-thread-lifecycle-spec.md`
- `docs/specs/code-graph-view-spec.md`
- `docs/specs/multi-repo-orchestration-spec.md`

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

Placement: After M10 web client/external gateway work and before M10.5 OKF Memory Bundle Foundation, M14 Hosted Alpha, and the broader M12 provider/harness backlog.

Tasks:

- OSYM-760 Harness Adapter And Capability Model
- OSYM-761 Model And Credential Settings
- OSYM-762 OpenHands Subscription Credential Adapter
- OSYM-763 Model Configuration UI And Routing Metadata
- OSYM-764 Codex App-Server Prototype And Benchmarks
- OSYM-766 ChatGPT OAuth For Codex Harness
- OSYM-767 Codex Production Harness Enablement
- OSYM-765 Codex Approvals And Cross-Harness Routing

## M10.4: Desktop Live Operations And Model Polish

Goal: Fix near-term desktop operator feedback from live Codex/OpenSymphony use before moving on to broader hosted and memory milestones.

Placement: After M10.3 Codex And Subscription Readiness and before M10.5 OKF Memory Bundle Foundation.

Tasks:

- OSYM-784 Desktop Live Snapshot And Run Detail Refresh
- OSYM-769 Run Detail Metrics And Density
- OSYM-780 Model Configuration Codex Subscription Follow-Up
- OSYM-782 TUI Codex Token Usage Accounting
- OSYM-783 Codex Event Content Summaries

## M10.5: OKF Memory Bundle Foundation

Goal: Evolve project memory into OKF-conformant bundles while keeping existing memory query, docs sync, and privacy behavior intact.

Tasks:

- OSYM-800 OKF Bundle Schema And Legacy Capsule Mapping
- OSYM-801 OKF Writer, Lint, And Migration Fixtures
- OSYM-802 Catalog Reindex And Query Compatibility From OKF
- OSYM-803 OKF Export, Import, And Visibility Boundaries
- OSYM-804 Docs Sync And MCP Admin Parity For OKF

## M10.6: Desktop Run Detail Operations And Interrupts

Goal: Make desktop run operations truthful and useful by adding real harness interruption, Human Review to Merging supersede handling, Run Detail action cleanup, TUI-parity run data, and a lazy desktop launcher command.

Tasks:

- OSYM-805 Harness Interrupt Contract And Run Diagnostics
- OSYM-806 OpenHands Agent-Server Interrupt Adapter
- OSYM-807 Codex App-Server Turn Interrupt Adapter
- OSYM-808 Merging Supersedes Human Review Polling
- OSYM-809 Desktop Run Detail Action Wiring And Cleanup
- OSYM-810 Desktop Run Detail TUI Parity
- OSYM-811 Lazy Desktop Launcher Command
- OSYM-812 Desktop Operations Integration Hardening

## M10.7: Project Grouping And Dependency Signals

Goal: Let operators scan active work by Linear project in the TUI and desktop app while keeping dependency signals compact and read-only.

Tasks:

- OSYM-813 Project Metadata For Operator Issue Snapshots
- OSYM-814 FrankenTUI Project Headers And Dependency Gutter
- OSYM-815 Desktop Project Grouping And Collapse
- OSYM-816 Project Grouping Integration Hardening

## M14: Hosted Alpha

Goal: Add hosted multi-user execution where server-owned runs continue after clients disconnect and permissions, secrets, workspaces, audit, and administration are enforced centrally.

Tasks:

- OSYM-750 Hosted Identity, Auth, And RBAC
- OSYM-751 Hosted Secrets And Linear Connections
- OSYM-752 Hosted Workspace Isolation And Runtime Pool
- OSYM-753 Client-Independent Run Persistence
- OSYM-754 Hosted Audit, Metrics, And Admin Controls
- OSYM-755 Hosted Subscription Credential Broker And Secret Store

## M11.5: LLM Wiki Graph View

Goal: Add a shared web and desktop OKF Knowledge Graph with gateway DTOs, graph extraction, community detection, an accessible inspector, and live privacy-aware memory integration.

Tasks:

- OSYM-820 Memory Graph DTOs And Gateway Endpoints
- OSYM-821 Graph Extraction, Metrics, And Community Pipeline
- OSYM-822 Shared Knowledge Graph Frontend Package And Reducers
- OSYM-823 Knowledge Graph Renderer And Worker Layouts
- OSYM-824 Knowledge Graph Inspector, Search, Filters, And Accessibility Fallback
- OSYM-825 Live Memory Graph Integration And Privacy Gates
- OSYM-826 Graph Scale, Visual Regression, And Web/Desktop Hardening

## M12: Provider, Harness, And Model Readiness

Goal: Hold follow-on provider, harness, and model readiness work after the near-term Codex subscription-readiness milestone.

Tasks:

- Follow-on tasks to be assigned after M10.3 Codex And Subscription Readiness.

## M13: ACP Debugging And IDE Attach

Goal: Expose OpenSymphony issue debug sessions through Zed ACP while preserving issue workspace, OpenHands conversation, and debug stream ownership in OpenSymphony.

Tasks:

- OSYM-840 Debug Attachment Core Refactor
- OSYM-841 ACP Stdio Server Protocol Adapter
- OSYM-842 Zed Static Agent Configuration And Setup UX
- OSYM-843 Tauri Debug-In-Zed Launch Action
- OSYM-844 Default Debug UX Transition And CLI Compatibility
- OSYM-845 ACP Debug Integration Tests And Failure Guidance

## M12.6: Tree-sitter Code Intelligence

Goal: Add a trusted Tree-sitter AST layer that gives agents source-cited structural code context through memory, MCP, and optional CLI tools.

Tasks:

- OSYM-850 Tree-sitter Provider Skeleton And Rust Parsing
- OSYM-851 Memory Context AST Provider Integration
- OSYM-852 Query Packs For Supported Agent Languages
- OSYM-853 Code Intelligence Persistence And Ingestion
- OSYM-854 Read-Only AST MCP And CLI Tools
- OSYM-855 Code Intelligence Performance Docs And Hardening

## M12.7: Workflow Target Branch Configuration

Goal: Let target repositories choose `main`, `master`, `develop`, or another long-lived integration branch for OpenSymphony-generated agent workflow guidance without rewriting local workflow customizations.

Tasks:

- OSYM-856 Workflow Target Branch Model And Init Customization
- OSYM-857 Init Target Branch Prompt And Flag
- OSYM-858 Update Workflow Settings Mode
- OSYM-859 Template Docs And Settings Hardening

## M12.8: Desktop App Installer And Auto-Update

Goal: Make `opensymphony app` a user-friendly first-run desktop installer and launcher with a configurable install root, verified bundle downloads, source-build fallback, prerequisite handling, and default-yes auto-update prompts.

Tasks:

- OSYM-860 Desktop Installer Contract And Release Metadata
- OSYM-861 Desktop Release Bundle Pipeline
- OSYM-862 App Download Install And Launch Flow
- OSYM-863 Source Build Fallback And Prerequisites
- OSYM-864 Desktop Auto-Update Flow
- OSYM-865 Installer Docs And End-To-End Validation

## M12.85: Codex Thread Lifecycle

Goal: Keep one recoverable Codex app-server thread per issue by reusing its
manifest-backed id across retries, retaining terminal workspaces, and archiving
or unarchiving that same thread at terminal and debug boundaries.

Tasks:

- OSYM-877 Canonical Codex Thread Reuse And Workspace Retention
- OSYM-878 Durable Codex Thread Archive And Debug Recovery

## M12.9: Code Graph View

Goal: Add a shared web and desktop Code Graph surface over Tree-sitter code-intelligence data, with stable symbol identity, gateway/native contracts, run-diff entry points, cross-graph chips, and scale/accessibility hardening.

Tasks:

- OSYM-870 Workspace Shell Graph Hero And Surface State
- OSYM-871 Symbol Identity Container Chain And Code Read Model
- OSYM-872 Code Graph DTOs Gateway Routes And Native Commands
- OSYM-873 Code Graph Frontend Surface Adapters And Inspector
- OSYM-874 Run Diff Symbol Navigation And Code Overlay
- OSYM-875 Cross Graph Code Memory And Work Chips
- OSYM-876 Code Graph Scale Accessibility And Parity Hardening
- OSYM-879 Target Branch Code Index And Revision Snapshots
- OSYM-880 Workspace Code Overlay And Composite Graph
- OSYM-881 Indexed Agent Code Context And Retrieval
- OSYM-882 Edge Delta And Module Topology Diff
- OSYM-883 Code Graph Bootstrap UX And End-To-End Validation

## M12.95: Multi-Repository Foundations

Goal: Establish central instance configuration, canonical terminal-task routing,
verified repository-specific execution, and one scoped per-instance memory
service without changing legacy single-repository behavior by default.

Tasks:

- OSYM-884 Central Multi-Repository Config And Safe Migration
- OSYM-885 Canonical Repository Binding And Task Propagation
- OSYM-886 Verified Checkouts Instructions And Harness Envelopes
- OSYM-887 Per-Instance Memory Catalog And Source Migration
- OSYM-888 Scoped Cross-Repository Memory And Leaf Overlays

## M12.96: Parent Integration Lifecycle

Goal: Retain terminal child evidence and let repository-neutral parents reuse
verified child storage for restart-safe integration, repairs, and lease-aware
bottom-up cleanup.

Tasks:

- OSYM-889 Hierarchy Generations And Ancestor Workspace Leases
- OSYM-890 Parent Execution Roots And Child Workspace Reuse
- OSYM-891 Restart-Safe Parent Integration Controller
- OSYM-892 Parent Repair Review And Merge Lifecycle
- OSYM-893 Bottom-Up Subtree Cleanup And Recovery

## M12.97: Multi-Repository Operations And Rollout

Goal: Make multi-repository state truthful across operator surfaces and prove
the entire lifecycle through hermetic fault injection before isolated
activation.

Tasks:

- OSYM-894 Multi-Repository Control Plane And Operator Surfaces
- OSYM-895 Hermetic Lifecycle Validation And Isolated Rollout

## M15: Hardening And Release Quality

Goal: Prove the system through contract, end-to-end, performance, security, accessibility, and documentation work.

Tasks:

- OSYM-770 Contract And Local End-To-End Tests
- OSYM-771 Web, Hosted, And Performance Tests
- OSYM-772 Security, Accessibility, Documentation, And Developer Experience
