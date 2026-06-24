# FrankenTUI Operator UI

## 1. Role of the UI

FrankenTUI is the optional human-readable status surface for OpenSymphony.

It must not be required for:

- orchestration correctness
- worker lifecycle
- retries
- reconciliation
- recovery

The daemon stays authoritative. FrankenTUI observes and renders.

## 2. Why FrankenTUI fits this project

FrankenTUI is a strong match for OpenSymphony because it emphasizes:

- diff-based deterministic rendering
- inline mode that preserves terminal scrollback
- one-writer terminal discipline
- RAII cleanup
- pane workspace layouts

Those qualities map well to a long-running orchestration dashboard with concurrent issue runs and live logs.

## 3. UI data source

FrankenTUI should talk only to the local OpenSymphony control plane.

### Read-only MVP channels

- HTTP snapshot endpoint for initial state
- control-plane WebSocket or SSE stream for updates
- optional log-file tail view through the daemon, not by opening private files directly

Current implemented local contract:

- `GET /api/v1/snapshot`
- `GET /api/v1/control/events` as SSE with `snapshot` events carrying serialized `SnapshotEnvelope`
- `GET /healthz` for daemon liveness

The standalone control-plane server still accepts the legacy
`/api/v1/events` path, but `opensymphony run` serves the full gateway on the
same bind address and reserves `/api/v1/events` for the gateway event journal.

The implemented client treats the configured base URL as a service-root prefix, so
`http://proxy/opensymphony` and `http://proxy/opensymphony/` both resolve API requests under
`/opensymphony/...` instead of silently dropping the prefix.

### Explicitly out of scope

- direct connection to OpenHands WebSocket streams
- direct access to orchestrator internals
- sending mutations into daemon internals without a versioned API

## 4. MVP screens

Recommended initial screens:

## 4.1 Dashboard

Shows:

- daemon health
- local agent-server health
- running issue count
- retry queue count
- last poll tick
- aggregated token and cost metrics if available, including cache-read and
  provider-reported total token counters when present in the snapshot

## 4.2 Issue list

Columns:

- issue identifier
- title
- tracker state
- orchestrator runtime state
- last worker outcome
- last event time
- active conversation ID suffix
- workspace path suffix

The issue list may add a compact dependency gutter and inline dependency suffix
without adding issue-list rows. See
[TUI Dependency Gutter Specification](specs/tui-dependency-gutter-spec.md).

## 4.3 Issue detail

Shows selected issue:

- normalized issue snapshot
- workspace metadata
- conversation metadata
- retry metadata
- input, output, cache-read, and total token counters from the control-plane
  issue snapshot
- recent worker outcomes
- recent validation commands if recorded

## 4.4 Event timeline

Shows recent summarized runtime events such as:

- worker started
- workspace created
- WebSocket attached
- run started
- tool call summary
- completion or failure
- retry scheduled

## 4.5 Log pane

Shows structured log excerpts for the selected issue or subsystem.

## 5. Layout model

Recommended first layout:

```text
+---------------------------------------------------------------+
| Status bar                                                    |
+------------------------+--------------------------------------+
| Issue list             | Selected issue detail                |
|                        |                                      |
+------------------------+--------------------------------------+
| Recent events / logs                                          |
+---------------------------------------------------------------+
```

Use pane-based layout so future views can expand without redesign.

The implemented inline layout budgets rows per pane instead of truncating one giant body block.
That keeps the bottom timeline visible under long issue lists, preserves the selected issue when snapshot ordering changes, and windows the issue pane around the current selection so narrower split terminals still keep the selected row and detail visible.
Issue rows are rendered as compact single-line summaries so the default inline view can keep more issues visible before scrolling, while the detail pane still carries the full per-issue metadata.
The detail pane now shows a git-backed changed-file summary for the selected workspace, including
per-file `+/-` counts. In wide layouts the lower-right pane stays on conversation activity until an
operator expands a changed file, at which point it becomes a diff viewer for that file while the
lower-left pane keeps the file list visible for navigation.
That changed-file summary and diff now compare the selected workspace against the branch merge-base
with `main` (falling back to `origin/main` when needed), so the pane behaves more like a GitHub PR
“Files changed” view and includes both committed branch work and any uncommitted edits.
The conversation activity pane now uses the full retained recent-activity window from the
control-plane snapshot instead of dropping older entries after the first ten, and it lets the
split-pane fitter handle width trimming so event summaries use the available column width.
Long conversation summaries now wrap within the pane instead of being forced onto a single clipped
row, and the lower-right pane has its own focus state so operators can scroll conversation history
directly without stealing file-selection focus from the workspace detail pane.
When the lower-right pane is showing conversation activity, it now defaults to the latest visible
output and keeps following new events as they arrive until the operator scrolls upward; scrolling
back down returns the pane to that tail-following mode.
The styled operator layout now splits the upper and lower pane regions evenly so the workspace
detail and diff area get half of the terminal height instead of being squeezed into the bottom 40%.
The always-visible status line now leads with daemon and local agent-server health before the
connection and focus metadata so degraded runtime state is visible even when the issue list is
otherwise stable.
Rendered pane text is normalized to a single visual line before fitting so
newline-bearing snapshot fields do not silently spill past the row budget.
Row fitting also uses terminal cell width instead of Unicode scalar counts and normalizes control
characters such as tabs before measurement so externally sourced tracker titles or event summaries
do not bleed across pane separators in split layouts.

## 6. Interaction model

MVP interaction should remain intentionally small.

Recommended commands:

- move selection
- cycle focus
- toggle the selected file diff in the detail pane
- switch between events and metrics
- quit cleanly

Current key map in the implemented client:

- `j` or down arrow: move selection down in the focused pane; detail focus moves through changed files, while the activity pane scrolls toward newer conversation output or down through the open diff
- `k` or up arrow: move selection up in the focused pane; detail focus moves through changed files, while the activity pane scrolls toward older conversation output or up through the open diff
- `tab`: cycle focus forward between the issue list, detail pane, and conversation or diff activity pane
- `shift-tab`: cycle focus backward through that same three-pane loop
- `enter`: toggle the diff for the currently selected changed file when detail or activity focus is active; opening a diff moves focus to the diff pane, and closing it returns the activity pane to conversation scrolling
- `e`: switch the bottom status pane between recent events and metrics without moving focus into it
- `q`: quit cleanly

The rendered status line and pane headers explicitly show the active focus target, and the top header also surfaces the computed connection, daemon, and agent-server cause text when bootstrap, reconnect, or degraded states need explanation.

Do not start with in-UI mutation commands unless the control plane already defines them cleanly.

## 7. Inline mode vs alternate screen

Default recommendation:

- use inline mode for day-to-day local monitoring
- support alternate screen as an option later if needed

Inline mode fits OpenSymphony because developers may want logs and UI to coexist in the same terminal session.

## 8. Rendering model

FrankenTUI should render from immutable view models produced by the control-plane client layer.

Pipeline:

1. fetch or receive new snapshot/event
2. reduce into TUI state
3. derive view model
4. render frame
5. let FrankenTUI diff and present

Avoid embedding business logic in widget code.

The control-plane bridge should follow the UI lifecycle. If inline mode exits
early, including `--exit-after-ms` harness runs or terminal startup failures
after bridge startup, the background bridge must stop polling and tear down
cleanly with the app.

## 9. Suggested Rust crate boundary

`opensymphony-tui` should contain:

- control-plane client
- TUI app state
- reducers
- view model conversion
- FrankenTUI widget composition
- keybinding map

It should not contain:

- tracker client
- workspace manager
- direct OpenHands client
- orchestrator state structs with private mutation access

## 10. Error handling

UI requirements:

- survive daemon disconnects
- show stale-data indicator
- reconnect to control plane when possible
- never panic the terminal session on missing fields
- degrade gracefully if future metric fields are unavailable; the MVP snapshot always includes the `metrics` object

Current reconnect behavior:

- fetch the latest snapshot over HTTP on startup
- if `/api/v1/snapshot` accepts the connection but hangs without returning a body, fail that bootstrap or reconnect refresh within the bounded snapshot timeout and retry instead of waiting forever
- if `/api/v1/control/events` never finishes attaching, including streams that open headers and then only emit keepalive comments before the first snapshot, fail that attach attempt within one bounded stream-attach timeout and retry instead of waiting forever in `conn=connecting` or `conn=reconnecting`
- keep rendering that bootstrap snapshot with `conn=connecting` until the SSE stream yields its first snapshot
- publish the first streamed snapshot and the `conn=live` attachment signal atomically through the bridge mailbox so the header never outruns the data it is describing
- subscribe to the SSE stream
- if the stream closes or fails, keep the last good snapshot visible, mark the connection as reconnecting, and surface the computed reconnect reason in the top header
- if `/api/v1/control/events` goes silent for longer than the keepalive watchdog budget after the connection opens, treat that stalled stream as failed and retry instead of hanging forever on stale data
- refetch the current snapshot before resubscribing
- if that refresh succeeds before the SSE stream reattaches, keep `conn=reconnecting` but switch the compact header detail to the current snapshot state such as `refreshed; stream pending`
- while the FTUI owns terminal output, bridge reconnect failures stay inside the reducer and header state instead of printing duplicate warning lines to `stderr`
- if the SSE consumer lags, accept the latest published snapshot immediately instead of waiting for the retained SSE backlog to drain, and ignore any older retained sequence that would roll the UI backward

The implemented bridge between the SSE client and the FTUI reducer coalesces bursty snapshot traffic down to the latest value so inline-mode polling does not accumulate an unbounded backlog of stale snapshots.

## 11. Dependency strategy

The current implementation uses the published `ftui` facade from crates.io with the `crossterm` feature enabled.

This keeps the OpenSymphony workspace self-contained while preserving the option to reorganize the internal module tree later if a future FrankenTUI feature needs a different repository boundary.

## 12. Testing approach

Automated:

- reducer tests
- snapshot-to-view-model tests
- simple rendering smoke tests
- control-plane client reconnection tests

Current automated coverage:

- reducer selection and mode-switch tests
- render smoke tests against serialized snapshots, including visible focus markers, selection persistence across snapshot reordering, selected-row visibility in truncated issue panes, narrow-layout detail preservation, and persistent bottom-pane visibility
- newline-normalization coverage for externally sourced event text so pane row counts stay accurate
- control-plane snapshot plus SSE round-trip tests
- TUI reconnect retention and narrow-layout detail visibility tests
- bridge and control-plane catch-up tests for snapshot coalescing, disconnect handling, and lagged SSE recovery

Manual:

- dashboard on multiple concurrent issues
- long log output with inline mode
- terminal resize handling
- clean shutdown and terminal restoration

## 13. Future extensions

Possible later additions:

- issue search box
- richer grouping and sorting
- keyboard-driven inspection of workspace artifacts
- control-plane mutation commands
- hosted dashboard mode using the same snapshot model

Keep the MVP read-only and reliable first.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-255 contributed: PR #4: COE-255: add control plane and FrankenTUI slice
- COE-269 contributed: PR #23: COE-269 + COE-271: add control plane and FrankenTUI slice (merge `53773f9`)
- COE-271 contributed: PR #23: COE-269 + COE-271: add control plane and FrankenTUI slice (merge `53773f9`)
- COE-287 contributed: PR #48: Add opensymphony debug command for issue conversations (merge `021f5ad`)
- COE-321 contributed: PR #59: Show more issues in the TUI issue list (merge `d9bd68c`)
- COE-395 contributed: PR #88: COE-395: Expand planning artifact schema and session service (merge `c1d8be9`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-255: Observability and FrankenTUI
- COE-269: Control-plane API and snapshot store
- COE-271: FrankenTUI operator client
- COE-287: Add opensymphony debug command for conversational session debugging
- COE-321: Add more lines to TUI issue list for better visibility
- COE-395: Planning Artifact Schema And Session Service
- COE-406: Repository, Linear, And Research Analysis
- COE-413: Implementation Plan Generator Stage
- COE-415: Milestone, Issue, And Sub-Issue Compiler
- COE-416: Dependency Graph And Plan Checks
- COE-417: Planning Workspace UI

## Source refs

- COE-255
- COE-269
- COE-271
- COE-287
- COE-321
- COE-395
- COE-406
- COE-413
- COE-415
- COE-416
- COE-417

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
