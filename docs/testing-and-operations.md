# Testing

This document defines the test strategy for OpenSymphony. For local operating
procedures, doctor guidance, rehydration, diagnostics, packaging, and safety
notes, see [operations.md](operations.md).

## 1. Testing philosophy

OpenSymphony sits at the intersection of:

- a specification-driven orchestrator
- an external issue tracker
- a remote-style agent runtime
- a terminal UI

The project needs more than unit tests. It needs layered validation with deterministic fakes and opt-in live tests.

## 2. Test layers

## 2.1 Unit tests

Every internal subsystem module should have focused unit tests for pure logic.

Examples:

- workflow parsing and strict template rendering
- issue identifier sanitization
- config resolution and environment indirection
- retry delay math
- event ordering and deduplication
- snapshot reducers
- TUI reducers and formatting helpers

## 2.2 Contract tests

Use the internal `opensymphony_testkit` module for protocol-level checks
against stable fixtures.

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

- `cargo check-dev`, `cargo test-dev`, and `cargo clippy-dev` are repository
  aliases for iterative OpenSymphony development. The aliases set
  `DUCKDB_DOWNLOAD_LIB=1` only for the aliased command and build with
  `--no-default-features --features duckdb-prebuilt` so the native DuckDB
  library is downloaded into
  `target/duckdb-download` and reused across rebuilds in the same target
  directory.
- `cargo test` exercises the full root package, including the fake-server contract suite from `tests/fake_server_contract.rs`
- `cargo test --test linear_client` exercises fixture-backed GraphQL normalization, parent/child hierarchy extraction, personal-API-key auth headers, required API-key/project/state configuration validation, issue URL/raw-priority preservation, full label pagination, raw workflow-state type preservation alongside normalized kinds, non-archived candidate polling, archived terminal cleanup reads, archived by-ID state refresh, GraphQL 400/429 rate-limit retries including reset-header handling, retryable 5xx GraphQL error envelopes, project-scoped by-ID state refresh, and tracker error mapping against a local stub server
- `cargo test --test hierarchy_selection --test scheduler` exercises blocker-aware and hierarchy-aware dispatch filtering, leaf-before-parent ordering, cached per-state capacity limiting, continuation retry, exponential failure backoff, runtime-event-fed stall detection, terminal cleanup/release, active-state reconciliation, and manifest-backed workspace recovery against fake tracker/workspace/worker backends
- `cargo test --lib orchestrator_run::backends::tests` covers runtime workspace-manifest recovery, in-flight run detection from `run.json`, and launch-path failure handling in the concrete CLI adapter
- `tests/doctor.rs` runs the CLI live-probe path against the internal `opensymphony_testkit` module
- `scripts/smoke_local.sh` runs the static doctor pass
- `scripts/live_e2e.sh` gates the live doctor run behind `OPENSYMPHONY_LIVE_OPENHANDS=1`
- `tests/fake_server_contract.rs` and `tests/client_resilience.rs` now split the runtime stream coverage intentionally: the shared fake-server contract suite owns the scripted initial snapshot replay, attach-backlog versus buffered-live ordering, reconnect exhaustion, explicit-close shutdown semantics, reconcile, out-of-order delivery, and reconnect recovery cases, while `client_resilience.rs` keeps the narrower auth, forward-compatibility, and mirror-regression cases that still need bespoke server behavior
- `tests/live_pinned_server.rs` provides an opt-in live integration check against the pinned `openhands-agent-server==1.24.0` surface for external-mode auth success and failure
- `tests/issue_session_runner.rs` now covers continuation reuse, already-running conversation wait/retry behavior, launch reporting for reused running conversations before prior-turn drain completes, delayed `/run` conflicts that surface an active prior turn only after attach, missing-conversation recreation that stays on continuation guidance, **simplified conversation resumption that reuses conversations as-is without LLM config drift checks**, configured `persistence_dir_relative` handling, terminal-error normalization, and temp-repo smoke execution
- `tests/supervisor.rs` now covers startup rejection when a foreign ready server is already bound to the supervised target port
- `tests/update.rs` covers the new `opensymphony update` maintenance flow: skipping `cargo install opensymphony` when the running CLI already matches the newest published release, running the install when a newer release exists, refreshing template-managed skill files in place for an existing target repo, and skipping that skill refresh outside a repo that lacks `WORKFLOW.md` plus `config.yaml`
- `tests/memory.rs` covers the first memory workflow: capture dry runs, capsule writes, DuckDB indexing, compact briefs, search, docs-sync dry-run diffs, public/private link handling, and Linear archive eligibility gating
- `opensymphony-gateway-schema/tests/gateway_schema.rs` and
  `opensymphony-gateway/tests/gateway.rs` cover Codex local readiness rendering
  with fake command outputs for installed, logged-out, unsupported, and
  permission-denied states. These tests assert that the gateway exposes only
  safe status metadata and `codex_cli_login` references, never raw OAuth access
  or refresh material.

Codex ChatGPT subscription smoke testing remains opt-in on trusted local
machines because the final exec probe can consume account quota. The supported
operator sequence is:

```bash
codex --version
codex app-server --help
codex login status
codex --ask-for-approval never exec --sandbox read-only \
  "Reply with exactly: CODEX_LOGIN_OK"
```

If login is missing or expired, use `codex login --device-auth`; if the account
has not enabled device-code authorization, enable ChatGPT Settings -> Security
and login -> Enable device code authorization for Codex before retrying.

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
- resolve `openhands.local_server.command` during workflow loading and honor it only for daemon-managed local supervision
- fail at runtime when `openhands.local_server.command` is configured for external, authenticated, or `local_server.enabled: false` targets
- fail when `openhands.local_server.enabled: false` is configured until the runtime supervisor can honor workflow-owned local-server disablement instead of still deciding launch behavior from the localhost base URL plus pinned tooling readiness
- fail when `openhands.local_server.env` is configured until the runtime supervisor creation path forwards workflow-owned launcher environment variables instead of always using runtime-owned defaults
- fail when `openhands.local_server.readiness_probe_path` is configured until the runtime supervisor launch path consumes workflow-owned probe settings instead of always using `/openapi.json`
- fail when `openhands.local_server.startup_timeout_ms` is configured until the runtime supervisor creation path consumes workflow-owned startup timeout settings instead of always using the supervisor default
- resolve the bundled `examples/target-repo/WORKFLOW.md` file end-to-end, not just parse it
- treat a leading unmatched `---` as prompt body text instead of failing front-matter parsing
- treat leading thematic-break-delimited non-mapping blocks as prompt body text instead of silently dropping prompt content
- fail on malformed, non-`http://`/`https://`, credential-bearing, query-bearing, fragment-bearing, or bracketed-IPv6 `openhands.transport.base_url` values during workflow resolution
- allow `https://` and path-prefixed OpenHands transport base URLs during workflow resolution
- fail when a non-loopback OpenHands transport base URL uses `http://`
- fail when a non-loopback OpenHands transport base URL omits `openhands.transport.session_api_key_env`
- resolve `openhands.transport.session_api_key_env`, `openhands.websocket.auth_mode`, and `openhands.websocket.query_param_name` into the runtime transport config
- normalize unauthenticated path-prefixed loopback OpenHands transport base URLs back to their origin before managed local supervisor startup while preserving configured prefixes for external or authenticated targets
- fail when `openhands.websocket.auth_mode` is invalid or requires a missing session API key env
- fail when explicit `openhands.websocket.enabled` is configured before the runtime readiness path can honor disabling the socket
- resolve `openhands.websocket.ready_timeout_ms`, `reconnect_initial_ms`, and `reconnect_max_ms` into the runtime readiness and reconnect budgets
- reject removed `openhands.mcp` config with a migration error that points users to `LINEAR_API_KEY` and the repo-local GraphQL helper assets
- resolve `openhands.conversation.reuse_policy` for runtime consumers instead of rejecting non-default values during workflow loading
- default required OpenHands conversation request fields such as `confirmation_policy` and `agent`, including `confirmation_policy.kind` when the block is present without an explicit kind
- fail when `openhands.conversation.confirmation_policy` includes options that cannot be represented in the current OpenHands request subset
- fail when `openhands.conversation.max_iterations` exceeds the downstream OpenHands `u32` request range
- fail when `openhands.conversation.agent.log_completions` or extra agent option keys are configured before the runtime conversation-create adapter can forward them
- fail when `openhands.conversation.agent.llm` is present without a non-empty `model`
- fail when `openhands.conversation.agent.llm` includes extra option keys before the runtime conversation-create adapter can forward them
- resolve `openhands.conversation.agent.llm.api_key_env` and `base_url_env` into the conversation-create payload at runtime
- fail when configured `openhands.conversation.agent.llm.api_key_env` or `base_url_env` names are missing or blank in the runtime environment
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
- external server path-prefix probes
- conversation creation
- initial REST sync
- WebSocket readiness barrier
- post-ready reconcile
- reconnect with backoff
- out-of-order event insertion
- terminal state detection
- conversation reuse for `per_issue`
- `fresh_each_run` reset/new-conversation behavior
- runtime rejection of unsupported reuse-policy values
- persisted policy-drift resets
- pinned-server auth success and failure paths
- reuse after an already-active turn or `/run` conflict
- recreation of a missing conversation with persisted history
- **simplified conversation resumption without LLM config drift checks**
- workflow-owned `persistence_dir_relative` mapping
- supervised-mode rejection of foreign ready servers

## 3.4 Orchestrator

- poll candidate sorting
- blocker-aware and hierarchy-aware dispatch eligibility
- claim and release transitions
- max concurrency
- failure retry backoff
- continuation retry at fixed delay
- stall detection
- active-state refresh
- terminal cleanup
- restart recovery from manifests

Current repository implementation:

- `tests/scheduler.rs` covers continuation retry, failure backoff, cached per-state dispatch limits across finish/stall/inactive/terminal/reconciliation transitions, runtime-event-fed stall detection, terminal reconciliation with cleanup, and manifest-backed workspace recovery using fake backends
- local restart validation should confirm that `opensymphony run` publishes a recovered snapshot before the first post-restart launch wave, so the TUI issue list repopulates even when reused conversations still take time to attach
- `crates/opensymphony-cli/src/orchestrator_run/backends.rs` covers immediate launch-failure cleanup and abort-on-drop cleanup for tracked runtime worker tasks in the production CLI adapter

## 3.5 Control plane and TUI

- snapshot derivation
- JSON serialization
- streaming update fanout
- read-only client invariants
- pane layout persistence
- event log rendering

Current implemented checks:

- snapshot serialization in `opensymphony-domain`
- parent/sub-issue tracker normalization and issue-ref terminal matching in `opensymphony-domain`
- forward-compatible snapshot decoding for unknown additive recent event kinds in `opensymphony-domain`
- forward-compatible snapshot decoding for unknown additive `daemon.state`, `runtime_state`, and `last_outcome` values in `opensymphony-control`
- control-plane HTTP plus SSE round-trip coverage in `opensymphony-control/tests/control_plane.rs`
- control-plane bootstrap snapshot timeout coverage in `opensymphony-control/tests/control_plane.rs`
- control-plane SSE connect-establishment timeout coverage in `opensymphony-control/tests/control_plane.rs`
- control-plane idle SSE timeout coverage in `opensymphony-control/tests/control_plane.rs`, including retry-in-place reconnect signaling
- control-plane post-disconnect reconnect-timeout reapplication coverage in `opensymphony-control/tests/control_plane.rs`
- control-plane monotonic lag-recovery coverage in `opensymphony-control/src/lib.rs`
- gateway compatibility coverage for `/healthz`, `/api/v1/snapshot`, `/api/v1/capabilities`, and `/api/v1/dashboard/snapshot` in `opensymphony-gateway/tests/gateway.rs`
- `opensymphony run` startup coverage that verifies the configured bind address exposes both health and gateway dashboard routes in `opensymphony-cli/tests/run.rs`
- TUI reducer, visible-focus rendering, selection preservation across reorder, long-list selection windowing, narrow-layout detail budgeting, snapshot coalescing, stale snapshot rejection, post-restart snapshot reset recovery, disconnect retention, and reconnect-to-live recovery coverage in `opensymphony-tui`

## 3.6 Shared client shell (web and desktop remote)

The web and desktop clients both mount the shared `OpenSymphonyApp` shell from `@opensymphony/ui-core` against the same `GatewayTransport` interface, so remote parity is structural. Implemented checks:

- app-shell mount smoke (`packages/ui-core/__tests__/app-shell.test.ts`): status, task graph, run detail, evidence, profile, and failed-connection rendering
- auth-aware placeholder states (`packages/ui-core/__tests__/auth-states.test.ts`): unauthenticated (sign-in), unauthorized (access denied), forbidden (access forbidden), organization/project selection placeholders, and local `auth_modes:["none"]` gateways rendering the dashboard with no login gate; recovery when the gateway later permits a read
- remote web/desktop parity (`packages/ui-core/__tests__/remote-parity.test.ts`): the shell renders the same core dashboard metrics, task graph nodes, run detail, planning workspace, and stream events in both `mode:"web"` and `mode:"desktop"` against an identical fixture transport
- gateway error classification (`packages/api-client/__tests__/gateway-errors.test.ts`): `HttpGatewayTransport` maps HTTP 401/403 (including a 403 with an explicit `error_code:"unauthorized"` body signal) to a classified `GatewayRequestError`, and `authStateFromError` maps it to an `AuthState` from `@opensymphony/gateway-schema`

### Evidence for UI/shell changes

The shell is pure DOM rendered by `renderOpenSymphonyApp` into a `jsdom` document, so the jest suites assert the rendered DOM directly (not mock return values). For the COE-419 auth placeholder states this means concrete runtime evidence of the rendered output:

- `data-opensymphony-app-shell="mounted"` root carries `data-auth-state` set to `unauthenticated` / `unauthorized` / `forbidden` / `open` for each scenario.
- `[data-testid="auth-placeholder"]` is present only in non-open states and carries the matching `data-auth-state`; `textContent` contains "Sign in required" (unauthenticated), "Access denied" plus "do not have permission" (unauthorized), and "Access forbidden" (forbidden).
- `[data-testid="auth-sign-in"]` appears only for `unauthenticated`; `[data-testid="auth-refresh"]` appears for `forbidden`/`unauthorized`; `[data-testid="auth-org"]`/`[data-testid="auth-project"]` render the organization/project selection surface.
- For local `auth_modes:["none"]` gateways, `[data-testid="auth-placeholder"]` is absent and `.os-task-graph-panel` renders with `data-auth-state="open"`.

These DOM assertions are the runtime evidence for the new user-facing states. A screenshot/video capture is not produced in this headless unattended environment; the assertions above exercise the real `renderAuthPlaceholder` / `renderViewContent` code paths end-to-end through the shared shell.

### Captured rendered DOM (real shell, jsdom)

The following is actual captured output from mounting the real shared shell (`renderOpenSymphonyApp`, `mode:"web"`) against a `MockGatewayTransport` that simulates a hosted gateway rejecting the snapshot, plus a local `auth_modes:["none"]` gateway. Reproduce with `npx jest packages/ui-core/__tests__/auth-states.test.ts` (and a temporary DOM-dump harness over `renderOpenSymphonyApp`).

LOCAL `auth_modes:["none"]` gateway (snapshot succeeds):
```
data-auth-state = open
auth-placeholder present = false
task-graph-panel present = true
<section class="os-panel os-task-graph-panel">...<div class="os-empty">No task graph loaded</div></section>
```

Hosted gateway, snapshot rejected with HTTP 401 (`unauthenticated`):
```
data-auth-state = unauthenticated
auth-placeholder present = true   (data-auth-state="unauthenticated")
auth-sign-in present = true  auth-refresh present = true  auth-org/project present = true
task-graph-panel present = false
<section class="os-panel os-auth-panel" data-testid="auth-placeholder" data-auth-state="unauthenticated">
  <div class="os-section-head"><h2>Sign in</h2><span>hosted</span></div>
  <p class="os-auth-message" data-testid="auth-message">Sign in required to view this OpenSymphony workspace.</p>
  <div class="os-auth-actions">
    <button data-auth-action="sign-in" data-testid="auth-sign-in">Sign in</button>
    <button data-auth-action="refresh" data-testid="auth-refresh">Retry</button>
  </div>
  <div class="os-auth-scope" data-testid="auth-scope">... Organization / Project selects ...</div>
</section>
```

Hosted gateway, snapshot rejected with HTTP 403 hard deny (`forbidden`):
```
data-auth-state = forbidden
auth-placeholder present = true   (data-auth-state="forbidden")
auth-sign-in present = false  auth-refresh present = true  auth-org/project present = true
<section class="os-panel os-auth-panel os-auth-denied" data-testid="auth-placeholder" data-auth-state="forbidden">
  <h2>Access forbidden</h2>
  <p class="os-auth-message">Access to this workspace is forbidden.</p>
  <button data-testid="auth-refresh">Retry</button>
  <div class="os-auth-scope" data-testid="auth-scope">... Organization / Project selects ...</div>
</section>
```

Hosted gateway, snapshot rejected with HTTP 403 carrying `error_code:"unauthorized"` (`unauthorized` permission denial):
```
data-auth-state = unauthorized
auth-placeholder present = true   (data-auth-state="unauthorized")
auth-sign-in present = false  auth-refresh present = true  auth-org/project present = true
<section class="os-panel os-auth-panel os-auth-denied" data-testid="auth-placeholder" data-auth-state="unauthorized">
  <h2>Access denied</h2>
  <p class="os-auth-message">You are signed in but do not have permission to view this workspace.</p>
  <button data-testid="auth-refresh">Retry</button>
  <div class="os-auth-scope" data-testid="auth-scope">... Organization / Project selects ...</div>
</section>
```

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
- per-request `/events/search` snapshots that differ across initial sync and post-ready reconcile
- per-connection WebSocket frame sequences so reconnect attempts can observe different ready/drop behavior
- late terminal events
- duplicated events
- out-of-order timestamps
- dropped WebSocket connections
- restart and reattach scenarios

## 5. Live local acceptance suite

The live local suite proves the MVP runtime path can execute on a prepared
developer machine against the pinned local OpenHands server.

Implemented entrypoints:

- `OPENSYMPHONY_LIVE_OPENHANDS=1 cargo test --test live_local_suite -- --ignored --nocapture --test-threads=1`
- `OPENSYMPHONY_LIVE_OPENHANDS=1 ./scripts/live_e2e.sh`

Required machine inputs:

- `uv`, `git`, `curl`, and the Rust toolchain
- `OPENSYMPHONY_OPENHANDS_MODEL`
- `OPENSYMPHONY_OPENHANDS_API_KEY` for the live `doctor` probe
- the provider environment expected by the pinned OpenHands server for normal
  issue-session runs; `scripts/live_e2e.sh` sets `OPENAI_API_KEY` from
  `OPENSYMPHONY_OPENHANDS_API_KEY` only when `OPENAI_API_KEY` is otherwise unset

The repository-owned script performs the full live flow:

- runs `opensymphony doctor --config examples/configs/local-dev.with-live-openhands.yaml --live-openhands`
- launches the pinned local OpenHands server on `OPENSYMPHONY_LIVE_SUITE_SERVER_PORT` (default `8010`)
- runs the ignored `live_local_suite` integration tests serially
- writes logs and scenario artifacts under `target/live-local/<timestamp>/` unless
  `OPENSYMPHONY_LIVE_SUITE_OUTPUT_ROOT` overrides the root

### Scenario A: checklist-driven issue lifecycle

- generate a temp target repo with repo-owned `WORKFLOW.md`, `AGENTS.md`, and a two-step checklist
- populate the issue workspace through the documented `after_create` clone hook
- run one issue through the real `WorkspaceManager` plus `IssueSessionRunner` path
- verify workspace creation, prompt capture, conversation creation, and a deterministic first-run assistant reply

Expected artifacts:

- `lifecycle/summary.json`
- `lifecycle/workspaces/COE-LIVE-273/notes/live-suite-checklist.md`
- `lifecycle/workspaces/COE-LIVE-273/.opensymphony/conversation.json`
- `lifecycle/workspaces/COE-LIVE-273/.opensymphony/generated/session-context.json`

Expected assertions:

- the first run uses the full workflow prompt
- the first run records the exact assistant reply `run 1: workspace-created`
- `.opensymphony/` manifests and prompt captures exist for debugging

### Scenario B: conversation reuse

- run the same issue a second time against the same workspace
- verify the default `per_issue` policy reuses the same `conversation_id`
- verify continuation guidance is selected instead of a second full prompt
- verify the second deterministic assistant reply appears only after the reused conversation resumes

Expected artifacts:

- `lifecycle/summary.json`
- `lifecycle/workspaces/COE-LIVE-273/.opensymphony/prompts/last-continuation-prompt.md`
- `lifecycle/workspaces/COE-LIVE-273/.opensymphony/logs/git-status-after.txt`

The `git-status-after.txt` artifact comes from the workflow `after_run` hook, so the suite also
proves that worker finalization is routing through the workspace manager's `finish_run` path.

Expected assertions:

- first and second runs report the same `conversation_id`
- `last_prompt_kind` in `conversation.json` becomes `continuation`
- the recorded assistant replies end with:

```text
run 1: workspace-created
run 2: conversation-reused
```

### Scenario C: WebSocket reconnect

- place a local fault-injecting proxy in front of the pinned server
- drop the first WebSocket connection immediately after the readiness barrier
- verify the client reconnects, reconciles, and still observes terminal completion

Expected artifacts:

- `reconnect/summary.json`
- `reconnect/proxy.log`

Expected assertions:

- the proxy records at least two websocket connections
- exactly one injected drop is recorded
- the terminal runtime status is still reached after reconnect
- the final message history includes `OpenSymphony reconnect probe OK`

## 6. Operations

Operational guidance now lives in [operations.md](operations.md).

That document covers:

- `opensymphony init`, `run`, `debug`, `doctor`, and `rehydrate`
- local operator workflows and validation commands
- doctor scope and live probe behavior
- logging, manifests, and recovery inspection
- version pinning, CI, and local safety posture

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-428 contributed: PR #133: Add alpha model configuration UI (merge `6f6b94e`)
- COE-475 contributed: PR #134: Expose Codex ChatGPT login readiness (merge `124ac40`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-428: Model Configuration UI And Routing Metadata
- COE-475: ChatGPT OAuth For Codex Harness

## Source refs

- COE-428
- COE-475

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
