# OpenSymphony

OpenSymphony is a Rust implementation of the OpenAI Symphony design that uses OpenHands agent-server as the coding-agent runtime and FrankenTUI as the optional terminal operator UI.

The first target is a local MVP for trusted developer machines:

- one Rust daemon owns Symphony orchestration
- one local OpenHands agent-server subprocess provides agent execution
- each issue gets its own deterministic workspace path and OpenHands `working_dir`
- OpenHands runtime events are consumed through a WebSocket-first client from day one
- Linear polling, retries, reconciliation, and workspace lifecycle remain Symphony responsibilities
- FrankenTUI is an observer over a local control plane, not the source of truth

## Why this shape

Symphony is explicitly a language-agnostic service specification and the upstream repository encourages people to build it in the language of their choice. The Elixir/Codex codebase is an experimental reference implementation, not the definition of Symphony. OpenHands agent-server exposes the right execution primitives for a direct Rust integration: conversation creation over HTTP, per-conversation `workspace.working_dir`, background `run`, event search for reconciliation, and real-time streaming over WebSocket. FrankenTUI is a strong fit for the optional status surface because it is designed around deterministic diff-based terminal rendering, inline mode, and pane workspaces.

## MVP scope

The MVP is intentionally local-first and trusted-environment-first.

Included:

- Symphony-faithful workflow loading from `WORKFLOW.md`
- typed config with an `openhands` extension namespace
- direct Linear read adapter for orchestration
- agent-side Linear writes via a small MCP server
- per-issue workspace manager with hooks and cleanup
- one local OpenHands agent-server process shared across issues
- one persistent OpenHands conversation per issue by default
- WebSocket-first runtime event handling with REST reconciliation
- read-only local control plane
- FrankenTUI client over the control plane
- deterministic tests plus live local integration tests

Not in MVP:

- multi-tenant hosted control plane
- centralized remote sandbox fleet
- browser UI
- replacing Symphony tracker polling with push webhooks
- implementing against the OpenHands web-app Socket.IO API

## Current implemented slice

This branch now boots the first local observability vertical slice even though the rest of the orchestration stack is still being filled in.

Available today:

- a Cargo workspace with shared snapshot domain models
- a read-only control-plane server with:
  - `GET /healthz`
  - `GET /api/v1/snapshot`
  - `GET /api/v1/events` as an SSE update stream
- a FrankenTUI client that:
  - fetches the initial snapshot over HTTP
  - reconnects to the SSE stream after disconnect
  - renders focused issue/workspace detail plus recent event or metrics panes in inline mode
  - shows the active focus pane in the status line and pane headers for keyboard-driven navigation
- a small `opensymphony-cli` demo path so the control plane and UI can be validated without coupling the TUI to orchestrator internals

Local commands:

- `cargo run -p opensymphony-cli -- daemon --bind 127.0.0.1:3000`
- `cargo run -p opensymphony-cli -- tui --url http://127.0.0.1:3000/`

## Core design decisions

### 1. Keep Symphony orchestration in Rust

Rust owns:

- poll loop
- runtime state
- workspace lifecycle
- retries and backoff
- Linear reconciliation
- status snapshots
- local control plane

OpenHands owns:

- agent execution
- tool use
- model provider access
- conversation persistence
- event generation

### 2. Treat the SDK agent-server API as the integration contract

The integration targets the SDK agent-server surface, not `openhands serve` and not the web app Socket.IO protocol. Operations are HTTP REST. Real-time updates are a plain WebSocket event stream.

### 3. Go WebSocket-first for agent updates

Symphony still polls Linear because the specification requires it. The change here is narrower: OpenHands agent-session updates use WebSockets first, with REST used for creation, command operations, recovery, and event reconciliation.

### 4. Use one conversation per issue by default

OpenSymphony persists a stable OpenHands `conversation_id` inside the issue workspace and reuses it across worker lifetimes. This is a deliberate implementation choice that preserves agent context across clean continuation runs while keeping the Symphony scheduler state in Rust.

### 5. Keep the UI optional

FrankenTUI is a consumer of the control-plane snapshot and event stream. The daemon must be fully correct without it.

## Document map

- `AGENTS.md`: persistent implementation rules for coding agents
- `WORKFLOW.example.md`: example repo workflow file with `openhands` extension config
- `docs/architecture.md`: high-level runtime design
- `docs/symphony-spec-alignment.md`: section-by-section mapping from Symphony spec to OpenSymphony
- `docs/openhands-agent-server.md`: chosen OpenHands integration surface
- `docs/websocket-runtime.md`: detailed WebSocket-first runtime contract
- `docs/workspace-and-lifecycle.md`: workspace layout, hooks, issue conversation policy
- `docs/linear-and-tools.md`: Linear read adapter and MCP write surface
- `docs/ui-frankentui.md`: operator UI design
- `docs/repository-layout.md`: crate ownership and repository boundaries
- `docs/deployment-modes.md`: local MVP mode and hosted follow-on mode
- `docs/testing-and-operations.md`: testing matrix, local ops, doctor checks
- `docs/sources.md`: primary references and trust notes
- `docs/implementation-plan.md`: milestone and dependency view
- `docs/tasks/`: issue-ready work items with Linear-friendly metadata

## Implementation milestones

### M1 Foundation and contracts
Workspace bootstrap, workflow/config loader, domain model, state machine.

### M2 OpenHands runtime adapter
Local server supervisor, REST client, WebSocket stream, session runner.

### M3 Symphony orchestration core
Workspace manager, Linear adapter, Linear MCP, orchestrator scheduler.

### M4 Operator UX and repo harness
Snapshot/control plane, FrankenTUI client, workspace-generated context artifacts.

### M5 Validation and local packaging
Fake agent-server, live local E2E suite, doctor command, packaging.

### M6 Hosted deployment follow-on
Remote agent-server mode, auth hardening, centralized deployment docs.

## Current bootstrap checks

The repository bootstrap keeps a compiling Rust workspace in place before the
runtime crates gain real behavior.

Current required checks:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets`
- `cargo test --workspace`

The local OpenHands tooling boundary lives in `tools/openhands-server/`. During
M1 it is intentionally fail-closed: the directory exists, the pin files are
reserved, and the launcher refuses to run until a validated package version and
lockfile are committed. Once those placeholders are replaced, the launcher uses
the pinned local `uv` environment and its `agent-server` extra instead of a
global `openhands` install.

## Local MVP quick-start for implementers

1. Read `AGENTS.md`.
2. Read `docs/architecture.md` and `docs/websocket-runtime.md`.
3. Implement milestone M1 before touching runtime code.
4. Build the OpenHands runtime adapter against a pinned server version.
5. Keep the control-plane API stable before expanding the TUI.
6. Use the task files in `docs/tasks/` as the Linear issue source of truth.

## Current local validation entrypoints

This repository now includes the local validation scaffolding for M5:

- a Rust workspace with the documented crate boundaries
- `opensymphony-openhands` for minimal conversation, search, and WebSocket readiness probes
- `opensymphony-linear-mcp` for a schema-tested Linear stdio MCP server
- `opensymphony-testkit` with an in-memory fake OpenHands server
- `opensymphony` CLI with a meaningful `doctor` command
- pinned OpenHands tooling under `tools/openhands-server/`
- example config and target-repo fixtures under `examples/`
- smoke and live validation scripts under `scripts/`

Useful commands:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p opensymphony-cli -- doctor --config examples/configs/local-dev.yaml
cargo run -p opensymphony-cli -- linear-mcp
./scripts/smoke_local.sh
OPENSYMPHONY_LIVE_OPENHANDS=1 ./scripts/live_e2e.sh
```

Current note:

- the example doctor YAML now only carries machine-local inputs such as the OpenHands tool directory and optional probe overrides; the target repo `WORKFLOW.md` provides the workspace root, OpenHands base URL, and prompt that the doctor probe validates
- `linear-mcp` is implemented and exposes the documented Linear tool surface over stdio; `daemon` and `tui` remain scaffolds until their runtime and control-plane milestones land.

## Non-negotiable implementation rules

- Do not collapse Symphony orchestration into OpenHands conversation state.
- Do not make FrankenTUI depend on internal orchestrator locks or structs.
- Do not implement against OpenHands web-app Socket.IO docs for this project.
- Do not assume WebSockets remove the need for REST reconciliation.
- Do not launch agent work outside the sanitized per-issue workspace path.
- Do not overwrite repository-owned `AGENTS.md` files inside target repos.
