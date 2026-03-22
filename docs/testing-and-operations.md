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
- `cargo test -p opensymphony-linear` exercises fixture-backed GraphQL normalization, personal-API-key auth headers, required API-key/project/state configuration validation, issue URL/raw-priority preservation, full label pagination, raw workflow-state type preservation alongside normalized kinds, non-archived candidate polling, archived terminal cleanup reads, non-archived by-ID state refresh, GraphQL 400/429 rate-limit retries including reset-header handling, retryable 5xx GraphQL error envelopes, project-scoped by-ID state refresh, and tracker error mapping against a local stub server
- `crates/opensymphony-cli/tests/doctor.rs` runs the CLI live-probe path against `opensymphony-testkit`
- `scripts/smoke_local.sh` runs the static doctor pass
- `scripts/live_e2e.sh` gates the live doctor run behind `OPENSYMPHONY_LIVE_OPENHANDS=1`
- `crates/opensymphony-openhands/tests/client_resilience.rs` and `crates/opensymphony-openhands/tests/fake_server_contract.rs` now cover readiness, attach, initial snapshot replay, attach-backlog versus buffered-live ordering, ready-barrier persistence across later stale rebuilds, explicit-close shutdown semantics, reconcile, out-of-order delivery, reused-conversation restart freshness, and reconnect recovery for the runtime stream

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
- normalize relative workflow directories first so relative `workspace.root` values still resolve to absolute paths
- reject parent-directory traversal in relative OpenHands persistence paths
- validate `openhands` extension namespace
- leave `openhands.local_server.command` unset when omitted so the runtime-owned local tooling layer resolves the pinned launcher from the OpenSymphony checkout
- fail when `openhands.local_server.command` is configured until the runtime supervisor can honor workflow-owned launcher overrides
- fail when `openhands.local_server.enabled: false` is configured until the runtime supervisor can honor workflow-owned local-server disablement instead of still deciding launch behavior from the localhost base URL plus pinned tooling readiness
- fail when `openhands.local_server.env` is configured until the runtime supervisor creation path forwards workflow-owned launcher environment variables instead of always using runtime-owned defaults
- fail when `openhands.local_server.readiness_probe_path` is configured until the runtime supervisor launch path consumes workflow-owned probe settings instead of always using `/openapi.json`
- fail when `openhands.local_server.startup_timeout_ms` is configured until the runtime supervisor creation path consumes workflow-owned startup timeout settings instead of always using the supervisor default
- resolve the bundled `examples/target-repo/WORKFLOW.md` file end-to-end, not just parse it
- treat a leading unmatched `---` as prompt body text instead of failing front-matter parsing
- treat leading thematic-break-delimited non-mapping blocks as prompt body text instead of silently dropping prompt content
- fail on malformed, non-`http://`, path-bearing, query-bearing, fragment-bearing, or bracketed-IPv6 `openhands.transport.base_url` values during workflow resolution
- fail when explicit `openhands.websocket.enabled`, `ready_timeout_ms`, `reconnect_initial_ms`, or `reconnect_max_ms` values are configured before the runtime readiness/reconnect path consumes them
- fail when `openhands.transport.session_api_key_env` or explicit OpenHands WebSocket auth knobs are configured before the runtime transport layer consumes them
- fail when `openhands.mcp.stdio_servers` is configured before the runtime conversation-create adapter can forward `mcp_config`
- fail when non-default `openhands.conversation.reuse_policy` values are configured before the orchestrator/runtime path can honor alternate conversation reuse behavior
- default required OpenHands conversation request fields such as `confirmation_policy` and `agent`, including `confirmation_policy.kind` when the block is present without an explicit kind
- fail when `openhands.conversation.confirmation_policy` includes options that cannot be represented in the current OpenHands request subset
- fail when `openhands.conversation.max_iterations` exceeds the downstream OpenHands `u32` request range
- fail when `openhands.conversation.agent.log_completions` or extra agent option keys are configured before the runtime conversation-create adapter can forward them
- fail when `openhands.conversation.agent.llm` is present without a non-empty `model`
- fail when `openhands.conversation.agent.llm` includes extra option keys before the runtime conversation-create adapter can forward them
- fail when `openhands.conversation.agent.llm.api_key_env` or `base_url_env` are configured before the runtime conversation-create adapter can forward them
- fail on malformed `agent.max_concurrent_agents_by_state` entries
- preserve the Markdown body exactly after the front matter terminator
- treat whitespace-only prompt bodies as absent so `DEFAULT_PROMPT_TEMPLATE` still applies

## 3.2 Workspace manager

- sanitize issue identifiers
- refuse path escape
- create and reuse workspace
- persist issue and run manifests
- persist conversation manifests
- persist stable prompt captures plus per-run prompt archives
- persist generated `issue-context.md` and `session-context.json`
- allow fresh `after_create` hooks to bootstrap clone/worktree flows before `.opensymphony/` exists
- retry failed first-time `after_create` hooks on the next `ensure`
- remember a successful first-time `after_create` before later metadata bootstrap steps so clone/worktree hooks are not rerun after a post-hook bootstrap failure
- reject sanitized-key collisions when an existing current-path issue manifest belongs to another issue
- ignore foreign, copied, or undecodable `.opensymphony/issue.json` artifacts when deciding whether first bootstrap already completed
- hook timeout
- kill spawned hook descendants when a timeout fires
- hook stderr capture
- avoid login-shell startup files when launching Unix hooks
- reject symlinked workspace roots during reused-workspace validation
- reject symlink-based `cwd` escapes for hooks
- reject symlinked `.opensymphony` manifest reads and writes
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

Current implemented checks:

- snapshot serialization in `opensymphony-domain`
- forward-compatible snapshot decoding for unknown additive recent event kinds in `opensymphony-domain`
- forward-compatible snapshot decoding for unknown additive `daemon.state`, `runtime_state`, and `last_outcome` values in `opensymphony-control`
- control-plane HTTP plus SSE round-trip coverage in `opensymphony-control/tests/control_plane.rs`
- control-plane bootstrap snapshot timeout coverage in `opensymphony-control/tests/control_plane.rs`
- control-plane SSE connect-establishment timeout coverage in `opensymphony-control/tests/control_plane.rs`
- control-plane idle SSE timeout coverage in `opensymphony-control/tests/control_plane.rs`, including retry-in-place reconnect signaling
- control-plane post-disconnect reconnect-timeout reapplication coverage in `opensymphony-control/tests/control_plane.rs`
- control-plane monotonic lag-recovery coverage in `opensymphony-control/src/lib.rs`
- TUI reducer, visible-focus rendering, selection preservation across reorder, long-list selection windowing, narrow-layout detail budgeting, snapshot coalescing, stale snapshot rejection, post-restart snapshot reset recovery, disconnect retention, and reconnect-to-live recovery coverage in `opensymphony-tui`

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
- verify workflow validation rejects non-default `openhands.conversation.reuse_policy` values until runtime support exists

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

The scripted `tui --exit-after-ms` smoke path now exits `0` only when the
final reduced control-plane state is still a real streamed
`live control-plane stream` state. If the control plane never becomes live,
or briefly becomes live before falling back to reconnecting again, the command
exits non-zero instead of reporting a false-positive healthy attach.

The rendered TUI header also carries the reducer-owned control-plane status
text so reconnect and attach state remain visible even while the operator is
focused on another pane. The `/healthz` endpoint reflects the daemon snapshot
state instead of always returning `ok`, so local smoke checks should confirm
that degraded or stopped snapshots surface through the endpoint.

When validating reconnect behavior, confirm that a newer post-restart snapshot
is accepted even if the reducer never saw an explicit `ConnectionLost`, and
that the TUI does not report `live control-plane stream` until the SSE stream
has actually begun delivering updates. Also confirm that a hung
`/api/v1/snapshot` request times out instead of stalling the bridge forever,
that a never-established `/api/v1/events` attach times out back into reconnect,
that an `/api/v1/events` stream which only reaches `Open` or flushes headers
without any bootstrap snapshot also times out on the short attach budget,
that an idle `/api/v1/events` read also flips the bridge into reconnecting
while the event-source retry stays in flight, that a later blackholed reopen
is still bounded by the attach timeout, that a queued reconnect plus recovery
snapshot still renders one reconnecting frame before returning to live, and
that additive `recent_events[].kind` values still decode into a usable snapshot
for the UI. For scripted smoke coverage, also confirm that an unreachable
control plane causes `opensymphony tui --exit-after-ms ...` to exit non-zero.
Current command set in this repository:

- `cargo run -p opensymphony-cli -- doctor --config examples/configs/local-dev.yaml`
- `cargo run -p opensymphony-cli -- doctor --config examples/configs/local-dev.with-live-openhands.yaml --live-openhands`
- `cargo run -p opensymphony-cli -- linear-mcp`
- `./scripts/smoke_local.sh`
- `OPENSYMPHONY_LIVE_OPENHANDS=1 ./scripts/live_e2e.sh`

## 7. Doctor checks

`opensymphony doctor` should be a serious preflight tool, not a superficial version printer.

Current implemented scope for OSYM-201 and OSYM-203:

- load and resolve the target repo `WORKFLOW.md` before any runtime probe
- render the workflow prompt with a synthetic issue shape during doctor preflight
- resolve the repo-local OpenHands wrapper metadata from `tools/openhands-server/`
- report pin readiness from `version.txt`, `pyproject.toml`, and `uv.lock`
- start the supervised local server when the pin is valid and the workflow-derived loopback base URL is down
- verify HTTP readiness on the workflow-derived loopback base URL
- create a temp conversation with workflow-derived OpenHands settings and attach the WebSocket runtime stream
- reconcile events before and after readiness
- send a real probe message that includes the rendered workflow prompt, trigger `/run`, and wait for a healthy terminal stream state
- stop the supervised child and report launch metadata

Required checks:

### Repository and config

- config file exists and parses
- target repo exists
- target repo contains `WORKFLOW.md`
- target repo `WORKFLOW.md` resolves against the current environment
- target repo prompt template renders against the current issue/attempt input shape
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

- the static doctor path checks config parsing, target-repo presence, workflow load/resolve/render, workspace-root creation from the workflow, loopback bind scope from the workflow OpenHands transport, pinned-tooling files, launcher metadata, and pin consistency across `version.txt`, `pyproject.toml`, and `uv.lock`
- the live doctor path additionally probes `GET /openapi.json`, creates a temp conversation using workflow-derived OpenHands conversation settings, attaches `RuntimeEventStream`, waits through non-readiness WebSocket traffic until the readiness barrier is observed, sends a doctor message that includes the rendered workflow prompt, triggers `/run`, and waits for a healthy terminal `execution_status` of `finished` after post-ready reconcile and reconnect-aware streaming, including terminal REST refresh fallback when a post-completion WebSocket reattach exhausts and one final scheduler-turn buffered drain before success is accepted
- once that live doctor path has already observed terminal success on the attached stream, it reuses the last successful stream-backed conversation snapshot instead of requiring a final `GET /api/conversations/{id}` that can flap during agent-server shutdown
- when the configured workflow loopback base URL is down but the repo-owned tooling pin is ready, the live doctor path temporarily starts the local supervised server on that port, uses it for the probe, then stops it again
- failure-only runtime events such as `ConversationErrorEvent` and terminal `execution_status` values like `error` or `stuck` fail the live doctor probe instead of counting as generic post-run activity, even when a later mirrored `finished` status is already present in the same drained batch
- `crates/opensymphony-openhands/tests/client_resilience.rs` locks in the runtime adapter regressions for pre-readiness WebSocket frames, authenticated REST/WebSocket requests, forward-compatible readiness envelopes, ready-state freshness after attach, ready-barrier persistence across later stale state rebuilds, buffered live frames outranking later attach replay items, explicit-close suppression of replay and reconnect, reused-conversation restart freshness over stale terminal REST state, forward-compatible `state_delta` mirror refresh, stale readiness snapshots not regressing newer probe state after reconnect, undecodable later persisted state updates not suppressing a usable ready barrier, terminal REST fallback after reconnect exhaustion, deferred reconnect after buffered delivery, non-replay of reconnect-only readiness barriers, next-turn probe error delivery after `finished`, and post-terminal probe success when a final REST refresh would fail
- `crates/opensymphony-openhands/tests/fake_server_contract.rs` locks in attach, initial snapshot replay, reconcile, out-of-order insertion, and reconnect recovery against `opensymphony-testkit`
- `crates/opensymphony-cli/tests/doctor.rs` locks in the doctor default target-repo fallback, workflow-driven runtime inputs, and the pinned launcher `cwd` behavior
- `crates/opensymphony-cli/tests/linear_mcp.rs` drives the real `opensymphony linear-mcp` child process through MCP initialization, tool listing, and comment/transition/link/state-list calls against a local fake Linear GraphQL server
- the current example configs carry machine-local tool/probe settings only; the repo-owned workflow now supplies the workspace root and OpenHands base URL that doctor validates
- the current example configs disable Linear by default so local runtime validation can succeed without tracker credentials when the workflow omits `tracker.api_key`

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
<issue_workspace>/.opensymphony.after_create.json
<issue_workspace>/.opensymphony/
  issue.json
  run.json
  conversation.json
  prompts/
    last-full-prompt.md
    last-full-prompt.json
    last-continuation-prompt.md
    last-continuation-prompt.json
  runs/
    attempt-0001/
      prompt-full-001.md
      prompt-full-001.json
  logs/
  generated/
    issue-context.md
    session-context.json
```

These files should make restart recovery explainable without scraping daemon memory. The root-scoped `after_create` receipt explains why a partially bootstrapped workspace will skip rerunning clone/worktree hooks, `run.json` should retain the latest hook/status evidence for the worker lifetime, and the prompt plus generated-context artifacts should make repo-policy precedence and the last dispatched prompt inspectable without reconstructing daemon state.

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

The current implementation follows that fail-closed rule: doctor and the local
supervisor validate the repo-owned pin files before launch and refuse to start
if the version file, direct dependency pin, and resolved lockfile drift apart.

Current repository pin:

- `openhands-agent-server==1.14.0`
- `openhands-sdk==1.14.0`
- `openhands-tools==1.14.0`
- `openhands-workspace==1.14.0`
- Python `3.12.x`
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
