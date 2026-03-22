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
- `GET /api/v1/events` as SSE with `snapshot` events carrying serialized `SnapshotEnvelope`
  - lagged consumers are snapped forward to the latest published sequence instead of replaying stale snapshots out of order
  - additive non-`snapshot` event names are ignored until the next `snapshot` so mixed-version clients do not tear down a healthy stream
- `GET /healthz` for daemon liveness, with the returned `status` reflecting the daemon snapshot state (`ok`, `starting`, `degraded`, `stopped` for known states, while additive unknown states are preserved as-is)

The inline header should always surface the reducer-owned control-plane status
text so operators can see whether the client is still connecting, live on the
stream, or retrying after a disconnect without changing focus panes.

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
- aggregated token and cost metrics if available

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

## 4.3 Issue detail

Shows selected issue:

- normalized issue snapshot
- workspace metadata
- conversation metadata
- retry metadata
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

Current inline-mode guarantees in the implemented client:

- the bottom events or metrics pane keeps reserved rows in the default 22-line inline layout
- the narrow stacked layout also reserves rows for selected issue and workspace detail instead of letting the issue list consume the whole frame
- long issue lists render as a moving window so the active selection stays visible instead of scrolling off the visible pane

## 6. Interaction model

MVP interaction should remain intentionally small.

Recommended commands:

- move selection
- cycle focus
- switch between events and metrics
- quit cleanly

Current key map in the implemented client:

- `j` or down arrow: move selection down
- `k` or up arrow: move selection up
- `tab`: cycle focus between issue list, detail, and timeline panes
- `e`: switch the bottom pane between recent events and metrics
- `q`: quit cleanly

The rendered status line and pane headers explicitly show the active focus target so inline-mode navigation stays understandable without a mouse or alternate screen.

The selected issue should stay anchored by identifier across live snapshot reordering so the detail pane does not jump to a different issue just because the list order changed.

When the issue list is taller than the visible pane, the rendered list should follow the active row so keyboard navigation always leaves a visible `>` marker in the issue pane.

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
- degrade gracefully if optional metrics are absent

Current reconnect behavior:

- fetch the latest snapshot over HTTP on startup
- bound each bootstrap snapshot fetch so a hung `/api/v1/snapshot` cannot stall reconnect forever
- bound SSE connection establishment so a blackholed or never-opening `/api/v1/events` attach falls back into reconnect instead of hanging forever
- keep that short SSE attach timeout active until the first decoded bootstrap snapshot arrives, so a stream that only reaches `Open` or flushes headers cannot suppress it
- bound SSE reads so a stalled `/api/v1/events` connection falls back into reconnect instead of hanging forever
- treat retryable SSE transport failures as reconnecting state immediately while the same event-source object retries in the background
- reapply that same SSE attach-timeout guard after retryable failures so a later `Open`-only or blackholed reopen also falls back into reconnect instead of hanging forever
- keep that bootstrap snapshot visible while the client is still connecting or reconnecting
- only report `live control-plane stream` after the SSE subscription is actually yielding stream data
- make scripted `opensymphony tui --exit-after-ms ...` runs fail unless the final reduced control-plane state is still `live` when the timeout expires
- if the stream closes or fails, mark the connection as reconnecting while keeping the last good snapshot visible, and preserve that reconnecting state through at least one rendered frame even if a recovery snapshot is already queued
- ignore regressing snapshots unless they are clearly newer post-restart snapshots with fresher publish and generation timestamps
- refetch the current snapshot before resubscribing
- tolerate additive `recent_events[].kind` values by preserving unknown kinds instead of rejecting the whole snapshot payload
- tolerate additive `daemon.state`, `issues[].runtime_state`, and `issues[].last_outcome` values by preserving unknown strings instead of rejecting the whole snapshot payload

## 11. Dependency strategy

The current implementation uses the published `ftui` facade from crates.io with the `crossterm` feature enabled.

This keeps the OpenSymphony workspace self-contained while preserving the option to move to a path dependency later if a future FrankenTUI feature requires unpublished workspace crates.

## 12. Testing approach

Automated:

- reducer tests
- snapshot-to-view-model tests
- simple rendering smoke tests
- control-plane client reconnection tests

Current automated coverage:

- reducer selection and mode-switch tests
- render smoke tests against serialized snapshots, including visible focus markers and narrow-layout detail preservation
- mailbox tests for snapshot coalescing and last-good-snapshot retention across disconnects
- control-plane snapshot plus SSE round-trip tests
- control-plane bootstrap snapshot timeout coverage
- control-plane SSE connect-establishment timeout coverage
- control-plane idle SSE timeout coverage
- scripted CLI attach coverage for healthy and never-live `--exit-after-ms` runs
- monotonic SSE lag-recovery tests for slow consumers
- snapshot decoding coverage for unknown additive recent event kinds
- snapshot decoding coverage for unknown additive `daemon.state`, `runtime_state`, and `last_outcome` values

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
