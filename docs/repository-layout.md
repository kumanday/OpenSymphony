# Repository Layout

This document records the repository structure and crate boundaries for the OpenSymphony implementation repo.

## 1. Top-level structure

```text
OpenSymphony/
  AGENTS.md
  README.md
  WORKFLOW.example.md
  Cargo.toml
  rust-toolchain.toml
  crates/
  docs/
  examples/
  scripts/
  tools/
  .github/
```

## 2. Crate ownership

## 2.1 `opensymphony-domain`

Purpose:

- shared domain types
- runtime enums
- scheduler state and transition helpers
- snapshot models
- config-independent constants

Keep it free of HTTP, WebSocket, and filesystem side effects.

Current M1 public surface:

- normalized `Issue` and `BlockerRef` models
- `RunAttempt`, `RetryEntry`, `RuntimeSession`, and `WorkerOutcome`
- `OrchestratorSnapshot` plus running/retry snapshot entries

## 2.2 `opensymphony-workflow`

Purpose:

- `WORKFLOW.md` loading
- front matter parsing
- strict prompt rendering
- core plus `openhands` config schema
- env and path resolution helpers

Current M1 public surface:

- `WorkflowDefinition` for raw front matter plus prompt body
- `WorkflowConfig` typed getters with defaults and OpenHands namespace validation
- `Workflow::render_prompt(issue, attempt)` with deterministic template failures

## 2.3 `opensymphony-workspace`

Purpose:

- workspace path resolution
- sanitization
- containment checks
- hook runner
- issue and conversation manifest helpers

## 2.4 `opensymphony-linear`

Purpose:

- Linear GraphQL adapter
- issue normalization
- pagination
- tracker reconciliation helpers

## 2.5 `opensymphony-linear-mcp`

Purpose:

- stdio MCP server for agent-side Linear writes

## 2.6 `opensymphony-openhands`

Purpose:

- local server supervisor
- REST client
- WebSocket event stream
- event cache and state mirror
- issue session runner
- protocol error mapping

This crate owns all OpenHands-specific transport details.

## 2.7 `opensymphony-orchestrator`

Purpose:

- poll tick
- scheduler actor and policy decisions over the shared state machine
- worker supervision
- retry queue
- cancellation and reconciliation
- snapshot derivation inputs

Current M1 public surface:

- `SchedulerConfig` typed scheduling policy
- `SchedulerState` claim, run, retry, reconciliation, stall-detection, and recovery transitions
- snapshot derivation through `SchedulerState::snapshot`

## 2.8 `opensymphony-control`

Purpose:

- local control-plane HTTP API
- control-plane update stream
- snapshot publication and serialization

## 2.9 `opensymphony-cli`

Purpose:

- `daemon`
- `tui`
- `doctor`
- `linear-mcp`
- config and path resolution entrypoints

## 2.10 `opensymphony-tui`

Purpose:

- FrankenTUI operator app
- control-plane client
- reducers
- rendering

## 2.11 `opensymphony-testkit`

Purpose:

- fake OpenHands agent-server
- fake Linear helpers
- integration fixtures
- protocol contract assertions

Current M1 public surface:

- downstream public-API smoke coverage proving other crates can compile against `domain`, `workflow`, and `orchestrator`

## 3. Tools and scripts

Recommended layout:

```text
tools/
  openhands-server/
    README.md
    pyproject.toml
    uv.lock
    run-local.sh
    version.txt
scripts/
  smoke_local.sh
  live_e2e.sh
  generate_issue_graph.sh
```

Why keep a `tools/openhands-server/` directory:

- pin the exact OpenHands Python package version
- document how the local supervised server is provisioned
- avoid relying on a globally installed moving target
- make `doctor` checks deterministic

## 4. Examples and fixtures

Recommended layout:

```text
examples/
  target-repo/
    WORKFLOW.md
    AGENTS.md
    .agents/skills/
  configs/
    local-dev.yaml
    local-dev.with-linear.yaml
```

Use examples for:

- a minimal target repository
- sample workflow files
- local development config

The example target repository is distinct from the OpenSymphony implementation checkout.
It intentionally does not include `tools/openhands-server/`, which remains owned by the
implementation repo.

## 5. Docs structure

```text
docs/
  architecture.md
  symphony-spec-alignment.md
  openhands-agent-server.md
  websocket-runtime.md
  workspace-and-lifecycle.md
  linear-and-tools.md
  ui-frankentui.md
  repository-layout.md
  deployment-modes.md
  testing-and-operations.md
  implementation-plan.md
  sources.md
  tasks/
```

## 6. Suggested ownership rules by directory

- `crates/opensymphony-openhands/`
  - only place that knows OpenHands endpoint paths and WebSocket auth details
- `crates/opensymphony-orchestrator/`
  - only place that owns mutable scheduler state
- `crates/opensymphony-workspace/`
  - only place that mutates workspace lifecycle and hook execution
- `crates/opensymphony-tui/`
  - only place that depends on FrankenTUI
- `tools/openhands-server/`
  - only place that pins Python runtime packaging for local server supervision

## 7. What should not live in the repo root

Avoid putting these directly in the root unless there is a very good reason:

- protocol-specific JSON payload fixtures
- random integration scripts without ownership
- ad hoc sample configs
- OpenHands wire contract notes that belong in docs
- target-repository test fixtures

## 8. Workspace manifests and generated artifacts

Generated issue-workspace artifacts belong in the target issue workspace under `.opensymphony/`, not in the OpenSymphony implementation repo.

The implementation repo should only contain:

- code
- documentation
- examples
- test fixtures

## 9. Dependency rules

- `opensymphony-orchestrator` currently depends only on `opensymphony-domain`; future runtime adapters should translate workflow, tracker, and OpenHands inputs at the boundary instead of leaking transport types into the scheduler
- `opensymphony-tui` depends only on `control` client models, not on orchestrator internals
- `opensymphony-openhands` must not depend on `opensymphony-tui`
- `opensymphony-linear-mcp` can share models with `opensymphony-linear` but must remain runnable as an independent command

## 10. CI guidance

Suggested checks:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets`
- `cargo test --workspace`
- selected live tests behind opt-in env vars
- docs link check if practical
- OpenHands version pin validation in `tools/openhands-server/`

## 11. Future hosted additions

When hosted mode is added, prefer adding small focused crates or modules such as:

- remote auth helpers
- deployment manifests
- hosted control-plane adapters

Do not contaminate the local MVP crates with speculative hosted-only branching if a clean boundary exists.
