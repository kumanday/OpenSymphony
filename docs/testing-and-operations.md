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

## 3. Minimum required test coverage by subsystem

## 3.1 Workflow and config

- parse valid `WORKFLOW.md`
- fail on invalid front matter
- fail on unknown template variables
- resolve defaults and env vars
- validate `openhands` extension namespace

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
- monotonic SSE delivery after lagged receivers
- read-only client invariants
- reconnect state that preserves the last good snapshot
- selection stability across snapshot reordering
- pane layout persistence
- narrow inline rendering that keeps the selected issue row and detail visible
- event log rendering

Current implemented checks:

- snapshot serialization in `opensymphony-domain`
- control-plane HTTP plus SSE round-trip coverage in `opensymphony-control/tests/control_plane.rs`
- control-plane lag-recovery monotonicity coverage in `opensymphony-control/src/lib.rs`
- TUI reducer, visible-focus rendering, reconnect-state retention, stable selection across snapshot reordering, and render smoke tests in `opensymphony-tui/tests/reducer.rs`
- TUI bridge mailbox coverage for snapshot coalescing, preserving the last good snapshot across disconnects, and narrow-layout detail coverage in `opensymphony-tui/src/lib.rs`

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

Current workspace commands:

- `cargo run -p opensymphony-cli -- daemon --bind 127.0.0.1:3000`
- `cargo run -p opensymphony-cli -- tui --url http://127.0.0.1:3000/`

The TUI treats `--url` as a control-plane service root. Path-prefixed deployments such as
`http://proxy/opensymphony` and `http://proxy/opensymphony/` both resolve API requests beneath
that prefix.

Possible helper commands later:

- `opensymphony debug openhands`
- `opensymphony inspect workspace <issue-id>`
- `opensymphony inspect conversation <issue-id>`

Current validation commands for the implemented observability slice:

- `cargo test`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo run -p opensymphony-cli -- daemon --bind 127.0.0.1:4010 --sample-interval-ms 250`
- `curl http://127.0.0.1:4010/api/v1/snapshot`
- `cargo run -p opensymphony-cli -- tui --url http://127.0.0.1:4010/ --exit-after-ms 1200`
- `curl http://127.0.0.1:4010/healthz`

When validating the control-plane stream locally, confirm that a reconnecting client still shows the last successful snapshot and that lagged consumers only advance to newer snapshot sequences.
When validating `opensymphony-cli tui --exit-after-ms ...`, also confirm the
control-plane bridge stops polling when the UI exits so the harness does not
leave a background thread behind.
When validating long issue queues, also confirm that moving the selection keeps
the highlighted row visible and that snapshot reordering preserves focus on the
same issue identifier.
When validating the sample daemon payload, also confirm that `metrics.running_issues`
and `metrics.retry_queue_depth` match the runtime states shown in the rendered issue list.

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
- a reconcile call succeeds

### External services

- Linear API key present when Linear mode is enabled
- MCP child process can start when enabled

### Environment quality

- warn if server binds beyond loopback in local mode
- warn if local mode is used with an obviously shared workspace root
- warn if required secrets are missing

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

During the M1 bootstrap task, the directory may contain explicit placeholders
for those files so the repository boundary exists before the local supervisor
lands. Those placeholders must fail closed and must not start a server until
the exact package version, uv dependency pin, and resolved lockfile are
committed. Once they are replaced, the quick run script should launch the
pinned server through the local `uv` environment and its `agent-server` extra,
explicitly setting `RUNTIME=process`, passing `--host 127.0.0.1`, and using a
configured `--port`.
The wrapper should reject extra agent-server CLI flags so local smoke runs stay
aligned with the daemon-managed single-server topology; `OPENHANDS_SERVER_PORT`
is the only supported runtime override, and the sandbox selection stays fixed to
host-process mode.

Do not rely on a random globally installed `openhands` binary.

## 11. CI strategy

Recommended CI stages:

1. lint and format
2. unit tests
3. contract tests with fakes
4. selected integration tests
5. optional nightly live tests on a controlled runner

The bootstrap repository baseline is smaller: every PR should at least run
`cargo fmt --check`, `cargo clippy --workspace --all-targets`, and
`cargo test --workspace`.

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
