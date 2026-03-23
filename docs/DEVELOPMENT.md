# Development Guide

This document is for developers contributing to OpenSymphony. For user-facing documentation, see the [README](../README.md) and [docs/](./) directory.

## Repository Structure

```
OpenSymphony/
├── crates/                    # Rust workspace crates
│   ├── opensymphony-cli/     # CLI entrypoints
│   ├── opensymphony-control/ # Control plane API
│   ├── opensymphony-domain/  # Domain models and state machine
│   ├── opensymphony-linear/  # Linear GraphQL adapter
│   ├── opensymphony-linear-mcp/  # MCP server for Linear writes
│   ├── opensymphony-openhands/   # OpenHands runtime client
│   ├── opensymphony-orchestrator/ # Scheduler and orchestration
│   ├── opensymphony-testkit/     # Test fixtures and fakes
│   ├── opensymphony-tui/         # FrankenTUI client
│   ├── opensymphony-workflow/    # WORKFLOW.md parsing
│   └── opensymphony-workspace/   # Workspace lifecycle
├── docs/                      # Documentation
├── examples/                  # Example configs and target repo
├── scripts/                   # Validation scripts
├── tools/                     # Pinned tooling
│   └── openhands-server/     # Pinned OpenHands server
├── AGENTS.md                 # Coding agent guidelines
└── README.md                 # User-facing documentation
```

## Why This Shape

Symphony is explicitly a language-agnostic service specification. The Elixir/Codex codebase is an experimental reference implementation, not the definition of Symphony. OpenSymphony implements the specification in Rust with:

- Direct OpenHands agent-server integration (HTTP REST + WebSocket)
- Per-issue `workspace.working_dir` for isolation
- FrankenTUI for terminal UI (diff-based rendering, inline mode)

## Core Design Decisions

### 1. Keep Symphony Orchestration in Rust

Rust owns:
- Poll loop
- Runtime state
- Workspace lifecycle
- Retries and backoff
- Linear reconciliation
- Status snapshots
- Local control plane

OpenHands owns:
- Agent execution
- Tool use
- Model provider access
- Conversation persistence
- Event generation

### 2. Treat SDK Agent-Server API as Integration Contract

The integration targets the SDK agent-server surface, not `openhands serve` and not the web app Socket.IO protocol. Operations are HTTP REST. Real-time updates are plain WebSocket event stream.

Current local pin: `OpenHands/software-agent-sdk` `v1.14.0`, provisioned through `tools/openhands-server/`.

### 3. Go WebSocket-First for Agent Updates

Symphony still polls Linear because the specification requires it. The change here is narrower: OpenHands agent-session updates use WebSockets first, with REST used for creation, command operations, recovery, and event reconciliation.

### 4. Use One Conversation Per Issue by Default

OpenSymphony persists a stable OpenHands `conversation_id` inside the issue workspace and reuses it across worker lifetimes. This preserves agent context across clean continuation runs while keeping the Symphony scheduler state in Rust.

### 5. Keep the UI Optional

FrankenTUI is a consumer of the control-plane snapshot and event stream. The daemon must be fully correct without it.

## Implementation Milestones

### M1: Foundation and Contracts
Workspace bootstrap, workflow/config loader, domain model, state machine.

**Completed**: COE-252, COE-258, COE-259, COE-260

### M2: OpenHands Runtime Adapter
Local server supervisor, REST client, WebSocket stream, session runner.

**Completed**: COE-253, COE-261, COE-262, COE-265, COE-266

### M3: Symphony Orchestration Core
Workspace manager, Linear adapter, Linear MCP, orchestrator scheduler.

**Completed**: COE-254, COE-263, COE-264, COE-267, COE-268, COE-270, COE-277

### M4: Operator UX and Repo Harness
Snapshot/control plane, FrankenTUI client, workspace-generated context artifacts.

**Completed**: COE-255, COE-269, COE-271

### M5: Validation and Local Packaging
Fake agent-server, live local E2E suite, doctor command, packaging.

**Completed**: COE-256, COE-272, COE-273, COE-274

### M6: Hosted Deployment Follow-On
Remote agent-server mode, auth hardening, centralized deployment docs.

**Future work**: COE-257, COE-275, COE-276

## Development Workflow

### Bootstrap Checks

The repository bootstrap keeps a compiling Rust workspace in place before the runtime crates gain real behavior.

Current required checks:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test --workspace
```

### Local Validation Entrypoints

This repository includes the local validation scaffolding for M5:

- Rust workspace with the documented crate boundaries
- `opensymphony-openhands` for minimal conversation, search, and WebSocket readiness probes
- `opensymphony-linear-mcp` for a schema-tested Linear stdio MCP server
- `opensymphony-testkit` with an in-memory fake OpenHands server
- `opensymphony` CLI with a meaningful `doctor` command
- Pinned OpenHands tooling under `tools/openhands-server/`
- Example config and target-repo fixtures under `examples/`
- Smoke and live validation scripts under `scripts/`

### Useful Commands

```bash
# Format check
cargo fmt --check

# Clippy
cargo clippy --workspace --all-targets -- -D warnings

# Unit tests
cargo test --workspace

# Doctor (static validation)
cargo run -p opensymphony-cli -- doctor --config examples/configs/local-dev.yaml

# Doctor (with live OpenHands probe)
cargo run -p opensymphony-cli -- doctor --config examples/configs/local-dev.with-live-openhands.yaml

# MCP server
cargo run -p opensymphony-cli -- linear-mcp

# Smoke test
./scripts/smoke_local.sh

# Live E2E test
OPENSYMPHONY_LIVE_OPENHANDS=1 ./scripts/live_e2e.sh
```

### Testing Strategy

See [testing-and-operations.md](testing-and-operations.md) for the full test strategy.

Key test layers:

1. **Unit tests**: Pure logic in every crate
2. **Contract tests**: Protocol-level checks with `opensymphony-testkit`
3. **Integration tests with fakes**: CI-friendly deterministic tests
4. **Live local tests**: Opt-in tests against real OpenHands server

### Version Pinning

The local OpenHands server must be pinned inside `tools/openhands-server/`.

Include:
- Exact package version
- Lockfile
- Install instructions
- Quick run script
- Note about the exact WebSocket assumptions pinned by this repo

Current repository pin:
- `openhands-agent-server==1.14.0`
- `openhands-sdk==1.14.0`
- `openhands-tools==1.14.0`
- `openhands-workspace==1.14.0`
- Python `3.12.x`

Do not rely on a random globally installed `openhands` binary.

## Non-Negotiable Implementation Rules

- Do not collapse Symphony orchestration into OpenHands conversation state.
- Do not make FrankenTUI depend on internal orchestrator locks or structs.
- Do not implement against OpenHands web-app Socket.IO docs for this project.
- Do not assume WebSockets remove the need for REST reconciliation.
- Do not launch agent work outside the sanitized per-issue workspace path.
- Do not overwrite repository-owned `AGENTS.md` files inside target repos.

## Document Map

- `AGENTS.md`: Persistent implementation rules for coding agents
- `WORKFLOW.example.md`: Example repo workflow file with `openhands` extension config
- `docs/architecture.md`: High-level runtime design
- `docs/symphony-spec-alignment.md`: Section-by-section mapping from Symphony spec to OpenSymphony
- `docs/openhands-agent-server.md`: Chosen OpenHands integration surface
- `docs/websocket-runtime.md`: Detailed WebSocket-first runtime contract
- `docs/workspace-and-lifecycle.md`: Workspace layout, hooks, issue conversation policy
- `docs/linear-and-tools.md`: Linear read adapter and MCP write surface
- `docs/ui-frankentui.md`: Operator UI design
- `docs/repository-layout.md`: Crate ownership and repository boundaries
- `docs/deployment-modes.md`: Local MVP mode and hosted follow-on mode
- `docs/testing-and-operations.md`: Testing matrix, local ops, doctor checks
- `docs/sources.md`: Primary references and trust notes
- `docs/implementation-plan.md`: Milestone and dependency view
- `docs/tasks/`: Issue-ready work items with Linear-friendly metadata

## CI Strategy

Recommended CI stages:

1. lint and format
2. unit tests
3. contract tests with fakes
4. selected integration tests
5. optional nightly live tests on a controlled runner

Current repo workflow:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Contributing

1. Read `AGENTS.md` for coding guidelines
2. Read `docs/architecture.md` and `docs/websocket-runtime.md` for design context
3. Follow the crate boundaries in `docs/repository-layout.md`
4. Add tests for new functionality
5. Ensure `cargo test --workspace` passes
6. Update relevant documentation

## Failure Triage Guidelines

When a live failure happens, first classify it into one of these buckets:

- workflow/config error
- workspace lifecycle error
- OpenHands HTTP transport error
- OpenHands WebSocket stream error
- conversation state mismatch
- Linear API error
- scheduler logic error
- UI-only rendering issue

This prevents noisy bug reports that mix multiple layers together.
