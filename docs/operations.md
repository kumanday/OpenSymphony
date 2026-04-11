# Operations

This document covers local operator workflows, doctor checks, rehydration,
diagnostics, packaging, and local safety. For setup details, see
[configuration.md](configuration.md). For the test strategy, see
[testing-and-operations.md](testing-and-operations.md).

## Operational Commands

Recommended CLI commands for the repo:

- `opensymphony init`
- `opensymphony run`
- `opensymphony debug <issue-id>`
- `opensymphony tui`
- `opensymphony doctor`
- `opensymphony linear-mcp`
- `opensymphony rehydrate <issue-id> --reason "..."`

Recommended first-run sequence:

- `cargo install --path .`
- `./tools/openhands-server/install.sh`
- `opensymphony --help`
- `cd /path/to/target-repo && opensymphony init`
- review the copied `config.yaml` and update `openhands.tool_dir` if the
  template path does not match your machine
- `opensymphony doctor --config examples/configs/local-dev.yaml`

Current workspace commands:

- `cd /path/to/target-repo && opensymphony init`
- `opensymphony init` fetches the current bootstrap payload from the template
  repo's raw GitHub URLs, merges an existing `AGENTS.md`, prompts before
  overwriting other repo-owned files, and can optionally scaffold OpenHands AI
  PR review plus a local setup guide
- `cd /path/to/target-repo && opensymphony run`
- `cd /path/to/target-repo && opensymphony run --config ./config.yaml`
- `cd /path/to/target-repo && opensymphony debug COE-284`
- `cd /path/to/target-repo && opensymphony rehydrate COE-284 --reason "API key rotation"`
- `opensymphony tui --url http://127.0.0.1:2468/`

Possible helper commands later:

- `opensymphony inspect workspace <issue-id>`
- `opensymphony inspect conversation <issue-id>`

Current validation commands for the implemented orchestrator and observability
slice:

- `cargo test`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p opensymphony-cli --lib resolve_rehydrate_runtime_honors_root_config_tool_dir -- --nocapture`
- `cargo test -p opensymphony-cli --test init`
- `cargo test -p opensymphony-cli --test debug`
- `cargo install --path . --locked --root /tmp/opensymphony-install-check`
- `cd /path/to/target-repo && /tmp/opensymphony-install-check/bin/opensymphony init`
- `cd /path/to/target-repo && /tmp/opensymphony-install-check/bin/opensymphony run`
- `cd /path/to/target-repo && /tmp/opensymphony-install-check/bin/opensymphony debug COE-284`
- `curl http://127.0.0.1:2468/api/v1/snapshot`
- `opensymphony tui --url http://127.0.0.1:2468/ --exit-after-ms 1200`
- `curl http://127.0.0.1:2468/healthz`

The debug command is intentionally workspace-backed rather than tracker-backed:
it finds the managed issue workspace, loads `.opensymphony/conversation.json`,
and resumes the recorded OpenHands conversation from the original working
directory. For local-supervised workflows it reuses any ready server already
bound to the configured base URL and only launches the pinned repo-local server
when nothing ready is already serving there. Avoid leaving unrelated standalone
`openhands` CLI sessions on that port when validating orchestrator-managed
resume behavior.

The scripted `tui --exit-after-ms` smoke path now exits `0` only when the final
reduced control-plane state is still a real streamed `live control-plane
stream` state. If the control plane never becomes live, or briefly becomes live
before falling back to reconnecting again, the command exits non-zero instead
of reporting a false-positive healthy attach.

The rendered TUI header also carries the reducer-owned control-plane status text
so reconnect and attach state remain visible even while the operator is focused
on another pane. The `/healthz` endpoint reflects the daemon snapshot state
instead of always returning `ok`, so local smoke checks should confirm that
degraded or stopped snapshots surface through the endpoint.

When validating reconnect behavior, confirm that a newer post-restart snapshot
is accepted even if the reducer never saw an explicit `ConnectionLost`, and
that the TUI does not report `live control-plane stream` until the SSE stream
has actually begun delivering updates. Also confirm that a hung
`/api/v1/snapshot` request times out instead of stalling the bridge forever,
that a never-established `/api/v1/events` attach times out back into reconnect,
that an `/api/v1/events` stream which only reaches `Open` or flushes headers
without any bootstrap snapshot also times out on the short attach budget, that
an idle `/api/v1/events` read also flips the bridge into reconnecting while the
event-source retry stays in flight, that a later blackholed reopen is still
bounded by the attach timeout, that a queued reconnect plus recovery snapshot
still renders one reconnecting frame before returning to live, and that
additive `recent_events[].kind` values still decode into a usable snapshot for
the UI. For scripted smoke coverage, also confirm that an unreachable control
plane causes `opensymphony tui --exit-after-ms ...` to exit non-zero.

Current command set in this repository:

- `./tools/openhands-server/install.sh`
- `cargo install --path .`
- `cd /path/to/target-repo && opensymphony run --config ./config.yaml`
- if you are bootstrapping a new target repo, start by copying
  `examples/target-repo/config.yaml` to `./config.yaml` and then adjust
  `openhands.tool_dir`, bind port, and any env-backed paths as needed
- `curl http://127.0.0.1:2468/healthz`
- `curl http://127.0.0.1:2468/api/v1/snapshot`
- `opensymphony tui --url http://127.0.0.1:2468/ --exit-after-ms 1200`
- `opensymphony doctor --config examples/configs/local-dev.yaml`
- `opensymphony doctor --config examples/configs/local-dev.with-live-openhands.yaml --live-openhands`
- `OPENSYMPHONY_LIVE_OPENHANDS=1 cargo test -p opensymphony-openhands --test live_pinned_server -- --nocapture`
- `OPENSYMPHONY_LIVE_OPENHANDS=1 cargo test -p opensymphony-openhands --test live_local_suite -- --ignored --nocapture --test-threads=1`
- `opensymphony linear-mcp`
- `./scripts/smoke_local.sh`
- `OPENSYMPHONY_LIVE_OPENHANDS=1 ./scripts/live_e2e.sh`

When validating the local control-plane and TUI slice, also confirm that:

- the TUI header shows daemon and agent-server health even when the issue list
  itself does not change
- the TUI header also renders the computed connection and backend status text
  when bootstrap, reconnect, or degraded states need a cause string
- bootstrap and reconnect snapshot fetches time out within the bounded snapshot
  watchdog budget, so a hung `/api/v1/snapshot` response retries instead of
  pinning the UI in `connecting` or `reconnecting`
- `/api/v1/events` attach attempts time out within the bounded stream-attach
  watchdog budget, including streams that flush headers and then only emit
  keepalive comments before their first snapshot, so a blackholed, half-open,
  or first-message-stalled SSE attach retries instead of pinning the UI in
  `connecting` or `reconnecting`
- the bootstrap snapshot stays visible with `conn=connecting` until the SSE
  stream actually attaches
- the first streamed snapshot and the live-stream attachment signal are
  published atomically, so `conn=live` never appears while the frame is still
  rendering the older bootstrap or reconnect snapshot
- reconnecting clients keep the last good snapshot visible instead of
  regressing to stale state
- reconnecting clients switch the header detail to `refreshed; stream pending`
  once the HTTP refresh succeeds, even before the SSE stream is live again
- event-stream clients treat a connected-but-silent `/api/v1/events` transport
  as failed once it exceeds the keepalive watchdog budget, so the TUI retries
  instead of hanging forever on stale bootstrap data
- inline `opensymphony tui` reconnect failures stay inside the UI state and do
  not interleave raw bridge warning lines into terminal output
- lagged SSE consumers immediately fast-forward to the newest published
  snapshot instead of waiting for the retained broadcast backlog to go empty,
  and they only advance to newer snapshot sequences
- newline-bearing, control-character-bearing, or full-width tracker and event
  text stays within the pane row and column budget
- the demo-only `opensymphony daemon --sample-interval-ms ...` command keeps
  the initial `Starting` snapshot in place until the configured interval
  elapses

## Doctor Checks

`opensymphony doctor` should be a serious preflight tool, not a superficial
version printer.

Current implemented scope for OSYM-201 and OSYM-203:

- load and resolve the target repo `WORKFLOW.md` before any runtime probe
- render the workflow prompt with a synthetic issue shape during doctor
  preflight
- resolve the repo-local OpenHands wrapper metadata from
  `tools/openhands-server/`
- report pin readiness from `version.txt`, `pyproject.toml`, and `uv.lock`
- start the supervised local server when the pin is valid and the
  workflow-derived loopback base URL is down
- refuse to launch supervised mode when a different ready server is already
  answering on the configured loopback base URL
- verify HTTP readiness on the workflow-derived loopback base URL
- create a temp conversation with workflow-derived OpenHands settings and
  attach the WebSocket runtime stream
- reconcile events before and after readiness
- send a real probe message that includes the rendered workflow prompt,
  trigger `/run`, and wait for a healthy terminal stream state
- stop the supervised child and report launch metadata

Required checks:

### Repository and Config

- config file exists and parses
- required local commands (`cargo`, `curl`, `git`, and `uv`) are present on
  `PATH`
- required env-backed config placeholders resolve instead of silently
  collapsing to empty strings
- target repo exists
- target repo contains `WORKFLOW.md`
- target repo `WORKFLOW.md` resolves against the current environment
- target repo prompt template renders against the current issue/attempt input
  shape
- workspace root exists or can be created
- OpenHands version pin files and helper scripts exist in
  `tools/openhands-server/`

### Local Runtime

- Python environment for pinned OpenHands can be resolved
- supervised server command can start
- server responds on the expected base URL
- a test conversation can be created with a temp `working_dir`
- WebSocket can attach and reach readiness
- the doctor probe sends a real message and triggers `/run`
- a reconcile call succeeds after the probe run starts

### External Services

- Linear API key present when Linear mode is enabled
- MCP child process can start when enabled

### Environment Quality

- warn if server binds beyond loopback in local mode
- warn if local mode is used with an obviously shared workspace root
- warn if required secrets are missing

Current implementation notes:

- the static doctor path checks config parsing, target-repo presence, workflow
  load/resolve/render, workspace-root creation from the workflow, loopback bind
  scope from the workflow OpenHands transport, pinned-tooling files, launcher
  metadata, and pin consistency across `version.txt`, `pyproject.toml`, and
  `uv.lock`
- the static doctor path also checks that `cargo`, `curl`, `git`, and `uv` are
  present on `PATH`, so local operations docs and machine readiness stay
  aligned
- checkout-relative doctor defaults are derived from the config and tooling
  paths rather than the caller `cwd`, so running `opensymphony doctor` outside
  the repo root still validates the intended checkout and bundled
  `examples/target-repo`
- the live doctor and supervised local-server paths normalize the configured
  `openhands.tool_dir` to an absolute path before launch, so checked-in configs
  such as `examples/configs/local-dev.with-live-openhands.yaml` can keep
  repo-relative tooling paths without depending on the caller `cwd`
- doctor prints an explicit trusted-machine warning on every run and warns when
  the configured OpenHands target is not loopback, so local safety posture
  remains visible during setup and troubleshooting
- the live doctor path additionally probes `GET /openapi.json`, creates a temp
  conversation using workflow-derived OpenHands conversation settings, attaches
  `RuntimeEventStream`, waits through non-readiness WebSocket traffic until the
  readiness barrier is observed, sends a doctor message that includes the
  rendered workflow prompt, triggers `/run`, and waits for a healthy terminal
  `execution_status` of `finished` after post-ready reconcile and
  reconnect-aware streaming, including terminal REST refresh fallback when a
  post-completion WebSocket reattach exhausts and one final scheduler-turn
  buffered drain before success is accepted
- once that live doctor path has already observed terminal success on the
  attached stream, it reuses the last successful stream-backed conversation
  snapshot instead of requiring a final `GET /api/conversations/{id}` that can
  flap during agent-server shutdown
- when the configured workflow loopback base URL is down but the repo-owned
  tooling pin is ready, the live doctor path temporarily starts the local
  supervised server only for unauthenticated loopback root-path targets,
  switches follow-up probes to the launched supervisor base URL, uses it for
  the probe, then stops it again
- failure-only runtime events such as `ConversationErrorEvent` and terminal
  `execution_status` values like `error` or `stuck` fail the live doctor probe
  instead of counting as generic post-run activity, even when a later mirrored
  `finished` status is already present in the same drained batch
- missing `${VAR}` tokens in required or enabled-path config values now fail
  doctor during config expansion instead of silently validating the config
  directory as an empty fallback path
- optional live-only placeholders such as `openhands.probe_model` and
  `openhands.probe_api_key_env` are treated as unset when their env-backed
  overrides are absent, so shared configs can keep those overrides empty during
  the static doctor pass
- `crates/opensymphony-openhands/tests/client_resilience.rs` locks in the
  runtime adapter regressions for pre-readiness WebSocket frames, authenticated
  REST/WebSocket requests, forward-compatible readiness envelopes, ready-state
  freshness after attach, ready-barrier persistence across later stale state
  rebuilds, reused-conversation restart freshness over stale terminal REST
  state, forward-compatible `state_delta` mirror refresh, stale readiness
  snapshots not regressing newer probe state after reconnect, undecodable later
  persisted state updates not suppressing a usable ready barrier, terminal REST
  fallback after reconnect exhaustion, non-replay of reconnect-only readiness
  barriers, next-turn probe error delivery after `finished`, and post-terminal
  probe success when a final REST refresh would fail
- `crates/opensymphony-openhands/tests/fake_server_contract.rs` locks in
  attach, scripted initial snapshot replay, scripted attach-backlog versus
  buffered-live ordering, scripted reconnect exhaustion and explicit-close
  handling, reconcile, out-of-order insertion, reconnect recovery, and
  delete-plus-recreate behavior against `opensymphony-testkit`
- `crates/opensymphony-openhands/tests/live_pinned_server.rs` locks in
  external-mode auth success plus HTTP 401 and WebSocket 403 failure mapping
  against the pinned `openhands-agent-server==1.14.0` process
- `crates/opensymphony-cli/tests/doctor.rs` locks in the doctor default
  target-repo fallback outside the repo `cwd`, required-env placeholder
  failures, optional live-only placeholder tolerance during static runs,
  workflow-driven runtime inputs, and the pinned launcher `cwd` behavior
- `crates/opensymphony-cli/tests/linear_mcp.rs` drives the real
  `opensymphony linear-mcp` child process through MCP initialization, tool
  listing, and comment/transition/link/state-list calls against a local fake
  Linear GraphQL server
- the current example configs carry machine-local tool/probe settings only; the
  repo-owned workflow now supplies the workspace root and OpenHands base URL
  that doctor validates
- the current example configs disable Linear by default so local runtime
  validation can succeed without tracker credentials when the workflow omits
  `tracker.api_key`

## Rehydration

Rehydration is the explicit recreation of OpenHands conversations with history
preservation. Unlike automatic conversation reset (which was removed),
rehydration is an intentional operator action.

### When To Use Rehydration

- API key rotation: when the LLM API key has changed and you need to create new
  conversations with the new key
- corrupted conversation state: when a conversation's stored state is damaged
- LLM provider switch: when changing to a different model or provider

### Commands

```bash
# Rehydrate a single issue
opensymphony rehydrate COE-123 --reason "API key rotation"

# Rehydrate all conversations during doctor check
opensymphony doctor --config examples/configs/local-dev.yaml --rehydrate

# Rehydrate with custom summary size (default 50 events)
opensymphony doctor --config examples/configs/local-dev.yaml --rehydrate --max-summary-events 100

# Rehydrate without preserving conversation history (faster)
opensymphony doctor --config examples/configs/local-dev.yaml --rehydrate --no-summary
```

### How Rehydration Works

1. Reads the existing conversation manifest from `.opensymphony/conversation.json`
2. Builds a summary of the conversation history unless `--no-summary` is used
3. Deletes the old conversation from the OpenHands server
4. Creates a new conversation with the current LLM configuration
5. Seeds the new conversation with the summary as context
6. Persists the new conversation ID in the manifest

### Simplified Conversation Resumption vs Rehydration

- normal resumption: conversations are reused as-is without checking for LLM
  config drift. The stored configuration in the conversation's `meta.json` is
  used
- rehydration: explicitly deletes and recreates conversations with the current
  configuration. Use when you need to apply new API keys or switch providers

## Logging and Diagnostics

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

## Snapshot and Manifest Inspection

Each issue workspace should expose enough local artifacts to debug recovery:

```text
<issue_workspace>/.opensymphony.after_create.json
<issue_workspace>/.opensymphony/
  issue.json
  run.json
  conversation.json
  openhands/
    create-conversation-request.json
    last-conversation-state.json
  generated/
    session-context.json
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

These files should make restart recovery explainable without scraping daemon
memory. The root-scoped `after_create` receipt explains why a partially
bootstrapped workspace will skip rerunning clone/worktree hooks, `run.json`
retains the latest hook/status evidence for the worker lifetime,
`conversation.json` records issue ownership, reuse state, prompt-seeding
state, the `llm_config_fingerprint` now simplified to track only model name for
observability, and the persisted launch profile used by `opensymphony debug`.
The OpenHands plus generated snapshots preserve the exact create request,
latest mirrored conversation state, last dispatched prompt artifacts, and
latest normalized runner context without reconstructing daemon state.

## Version Pinning

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
aligned with the daemon-managed single-server topology;
`OPENHANDS_SERVER_PORT` is the only supported runtime override, and the sandbox
selection stays fixed to host-process mode.

The current implementation follows that fail-closed rule: doctor and the local
supervisor validate the repo-owned pin files before launch and refuse to start
if the version file, direct dependency pin, and resolved lockfile drift apart.
The packaged tool directory now also carries `install.sh`, which runs the
locked `uv sync --extra agent-server` flow that the README and doctor-guided
setup path expect.
The launcher itself now enforces `uv run --locked --extra agent-server`,
exports `RUNTIME=process`, only accepts `OPENHANDS_SERVER_PORT` as a runtime
override, and rejects extra agent-server CLI flags before `uv` is invoked.

Current repository pin:

- `openhands-agent-server==1.14.0`
- `openhands-sdk==1.14.0`
- `openhands-tools==1.14.0`
- `openhands-workspace==1.14.0`
- Python `3.12.x`

Do not rely on a random globally installed `openhands` binary.

## CI Strategy

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

## Local Safety Note

The MVP local mode runs agent activity on the host with process-level isolation.
The docs, CLI help, and doctor output should state this plainly.

The current `tools/openhands-server/run-local.sh` script binds OpenHands to
loopback by default, enforces the pinned `uv.lock` in host-process mode, and
the doctor command now always prints an explicit trusted-machine warning before
reporting readiness. It also warns when the configured base URL is not loopback
in local mode.
