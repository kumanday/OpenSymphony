# Testing and Operations

This document defines the test strategy, local operating model, and packaging guidance for OpenSymphony.

## 1. Testing philosophy

OpenSymphony sits at the intersection of:

- a specification-driven orchestrator
- an external issue tracker
- a remote-style agent runtime
- a terminal UI

The project needs more than unit tests. It needs layered validation with deterministic fakes and opt-in live tests.

## 2. Test layers

## 2.1 Unit tests

Every crate should have focused unit tests for pure logic.

Examples:

- workflow parsing and strict template rendering
- issue identifier sanitization
- config resolution and environment indirection
- retry delay math
- event ordering and deduplication
- snapshot reducers
- TUI reducers and formatting helpers

## 2.2 Contract tests

Use `opensymphony-testkit` for protocol-level checks against stable fixtures.

Required contract suites:

- conversation create payload serialization
- user-message event payload serialization
- `run` trigger request behavior
- WebSocket event decoding for known event types
- unknown-event pass-through handling
- event-search pagination and reconciliation
- terminal state derivation from `ConversationStateUpdateEvent`

## 2.3 Integration tests with fakes

Run these in CI.

Components to fake:

- OpenHands agent-server
- Linear GraphQL responses
- local control-plane API consumer

Why fakes matter:

- deterministic edge-case coverage
- out-of-order event sequences
- disconnect and reconnect behavior
- server restart scenarios
- scheduler recovery on daemon restart

## 2.4 Live local tests

These are opt-in and run against a pinned real OpenHands server on a trusted machine.

Gate them behind explicit environment variables.

Suggested gates:

- `OPENSYMPHONY_LIVE_OPENHANDS=1`
- `OPENSYMPHONY_LIVE_LINEAR=1`

Current implementation:

- `cargo test --workspace` exercises the fake-server contract suite in `crates/opensymphony-openhands/tests/fake_server_contract.rs`
- `crates/opensymphony-cli/tests/doctor.rs` runs the CLI live-probe path against `opensymphony-testkit`
- `scripts/smoke_local.sh` runs the static doctor pass
- `scripts/live_e2e.sh` gates the live doctor run behind `OPENSYMPHONY_LIVE_OPENHANDS=1`

## 3. Minimum required test coverage by subsystem

## 3.1 Workflow and config

- parse valid `WORKFLOW.md`
- parse the checked-in repository and example `WORKFLOW.md` files
- fail on invalid front matter
- fail on unknown top-level workflow namespaces
- fail on unknown template variables
- resolve defaults and env vars
- fail when an explicitly referenced env token such as `tracker.api_key: $VAR` is unset
- fall back to `LINEAR_API_KEY` when `tracker.api_key` is omitted
- fail when `tracker.active_states` or `tracker.terminal_states` are omitted
- resolve workflow-relative workspace paths and relative OpenHands persistence paths
- resolve bare relative workspace roots against the `WORKFLOW.md` directory
- reject parent-directory traversal in relative OpenHands persistence paths
- validate `openhands` extension namespace
- default `openhands.local_server.command` to the pinned `tools/openhands-server/run-local.sh` launcher
- default required OpenHands conversation request fields such as `confirmation_policy` and `agent`
- fail on malformed `agent.max_concurrent_agents_by_state` entries
- preserve the Markdown body exactly after the front matter terminator

## 3.2 Workspace manager

- sanitize issue identifiers
- refuse path escape
- create and reuse workspace
- hook timeout
- hook stderr capture
- cleanup on terminal issue state

## 3.3 OpenHands adapter

- supervised server startup and shutdown
- HTTP client auth modes
- conversation creation
- initial REST sync
- WebSocket readiness barrier
- post-ready reconcile
- reconnect with backoff
- out-of-order event insertion
- terminal state detection
- conversation reuse

## 3.4 Orchestrator

- poll candidate sorting
- claim and release transitions
- max concurrency
- failure retry backoff
- continuation retry at fixed delay
- stall detection
- active-state refresh
- terminal cleanup
- restart recovery from manifests

## 3.5 Control plane and TUI

- snapshot derivation
- JSON serialization
- streaming update fanout
- read-only client invariants
- pane layout persistence
- event log rendering

## 4. Fake OpenHands server requirements

The fake server in `opensymphony-testkit` should emulate the minimum runtime contract:

- `POST /api/conversations`
- `GET /api/conversations/{id}`
- `POST /api/conversations/{id}/events`
- `POST /api/conversations/{id}/run`
- `GET /api/conversations/{id}/events/search`
- `/sockets/events/{conversation_id}`

It should be scriptable enough to produce:

- clean success runs
- tool-heavy runs
- failure runs
- late terminal events
- duplicated events
- out-of-order timestamps
- dropped WebSocket connections
- restart and reattach scenarios

## 5. Live local acceptance suite

The live local suite should prove the MVP can actually run on a developer machine.

Suggested scenarios:

### Scenario A: workflow parse and local run smoke

- launch daemon
- start local supervised OpenHands server
- create temp target repo with example `WORKFLOW.md`
- inject one fake or test Linear issue
- verify workspace creation, conversation creation, run, and snapshot publication

### Scenario B: conversation reuse

- run the same issue twice
- verify the same `conversation_id` is reused
- verify continuation guidance is used instead of the full first-turn prompt

### Scenario C: WebSocket reconnect

- interrupt the WebSocket connection
- verify backoff, reattach, reconcile, and continued completion detection

## 6. Operational commands

Recommended CLI commands for the repo:

- `opensymphony daemon`
- `opensymphony tui`
- `opensymphony doctor`
- `opensymphony linear-mcp`

Possible helper commands later:

- `opensymphony debug openhands`
- `opensymphony inspect workspace <issue-id>`
- `opensymphony inspect conversation <issue-id>`

Current command set in this repository:

- `cargo run -p opensymphony-cli -- doctor --config examples/configs/local-dev.yaml`
- `cargo run -p opensymphony-cli -- doctor --config examples/configs/local-dev.with-live-openhands.yaml --live-openhands`
- `./scripts/smoke_local.sh`
- `OPENSYMPHONY_LIVE_OPENHANDS=1 ./scripts/live_e2e.sh`

## 7. Doctor checks

`opensymphony doctor` should be a serious preflight tool, not a superficial version printer.

Required checks:

### Repository and config

- config file exists and parses
- target repo exists
- target repo contains `WORKFLOW.md`
- workspace root exists or can be created
- OpenHands version pin files exist in `tools/openhands-server/`

### Local runtime

- Python environment for pinned OpenHands can be resolved
- supervised server command can start
- server responds on the expected base URL
- a test conversation can be created with a temp `working_dir`
- WebSocket can attach and reach readiness
- the doctor probe sends a real message and triggers `/run`
- a reconcile call succeeds after the probe run starts

### External services

- Linear API key present when Linear mode is enabled
- MCP child process can start when enabled

### Environment quality

- warn if server binds beyond loopback in local mode
- warn if local mode is used with an obviously shared workspace root
- warn if required secrets are missing

Current implementation notes:

- the static doctor path checks config parsing, target-repo presence, workspace-root creation, loopback bind scope, and pinned-tooling files
- the live doctor path additionally probes `GET /openapi.json`, creates a temp conversation, waits through non-readiness WebSocket traffic until the readiness barrier is observed, sends a probe prompt, triggers `/run`, and waits for a healthy terminal `execution_status` of `finished` before reconciling events
- failure-only runtime events such as `ConversationErrorEvent` and terminal `execution_status` values like `error` or `stuck` fail the live doctor probe instead of counting as generic post-run activity
- `crates/opensymphony-openhands/tests/client_resilience.rs` locks in the runtime adapter regressions for pre-readiness WebSocket frames and authenticated REST requests
- `crates/opensymphony-cli/tests/doctor.rs` locks in the doctor default target-repo fallback and the pinned launcher `cwd` behavior
- the current example configs disable Linear by default so local runtime validation can succeed without tracker credentials

## 8. Logging and diagnostics

Use structured logs everywhere.

Minimum fields:

- timestamp
- level
- subsystem
- issue identifier
- conversation ID
- worker attempt
- event type
- server base URL
- workspace path

Write logs to:

- stdout for normal operation
- optional rotating local file for debug mode

## 9. Snapshot and manifest inspection

Each issue workspace should expose enough local artifacts to debug recovery:

```text
<issue_workspace>/.opensymphony/
  issue.json
  conversation.json
  last-run.json
  prompts/
  logs/
```

These files should make restart recovery explainable without scraping daemon memory.

## 10. Version pinning

The local OpenHands server must be pinned inside `tools/openhands-server/`.

Include:

- exact package version
- lockfile
- install instructions
- quick run script
- note about the exact WebSocket assumptions pinned by this repo

Do not rely on a random globally installed `openhands` binary.

## 11. CI strategy

Recommended CI stages:

1. lint and format
2. unit tests
3. contract tests with fakes
4. selected integration tests
5. optional nightly live tests on a controlled runner

Current repo workflow:

1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`

## 12. Failure triage guidelines

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

## 13. Local safety note

The MVP local mode runs agent activity on the host with process-level isolation. The docs, CLI help, and doctor output should state this plainly.

The current `tools/openhands-server/run-local.sh` script binds OpenHands to loopback by default, and the doctor command warns when the configured base URL is not loopback in local mode.
