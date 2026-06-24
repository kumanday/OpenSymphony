# Desktop Run Detail Operations And Interrupt Specification

Date: 2026-06-24

Status: draft

## Purpose

Define the near-term desktop app improvement wave for run cancellation, debug and
workspace actions, TUI parity, and the lazy desktop launcher command.

This spec is intentionally scoped to the current single-conversation-per-task
model. When a Linear issue moves from `Human Review` to `Merging`, OpenSymphony
must interrupt the active turn or polling loop inside the same issue
conversation and continue toward landing or closeout. It must not invent a
second "Human Review worker", hand work to a different worker, or require a new
conversation unless a later harness contract explicitly requires that.

## Current Evidence

The current desktop run surface is partly ahead of the implemented control
plane:

- `packages/gateway-schema/src/run.ts` exposes `RunDetail.turn_count`,
  `max_turns`, `input_tokens`, `output_tokens`, `cache_read_tokens`,
  `conversation_id`, `workspace_path`, `harness_type`, `allowed_actions`, and
  `safe_actions`.
- `packages/ui-core/src/run-actions.ts` renders `Retry`, `Cancel`,
  `Rehydrate`, `Detach`, `Comment`, `Follow-up`, `Workspace`, and `Debug`
  whenever those actions appear allowed, even though some of those actions do
  not yet have real operator flows.
- `packages/ui-core/src/app-shell.ts` renders `Turns` as a fraction when
  `max_turns` is present and reserves panes for validation summaries and
  pending approvals.
- `packages/gateway-schema/src/snapshot.ts` already includes global
  `total_input_tokens`, `total_output_tokens`, and `total_cache_read_tokens`,
  but the desktop status pane currently shows only an input plus output total.
- The TUI displays useful run data that the desktop should mirror: branch, PR
  URL, per-run input/cache/output/total tokens, and green addition/red deletion
  file statistics.
- The installed OpenHands agent-server exposes
  `POST /{conversation_id}/interrupt`. Its router and service code distinguish
  this from `/pause`: interrupt cancels the in-flight request immediately and
  transitions the conversation to paused.
- The current Codex app-server schema exposes JSON-RPC `turn/interrupt` with
  required `threadId` and `turnId` params. `TurnStatus` includes
  `interrupted`. The current `opensymphony-codex` adapter still builds a stale
  `turn/cancel` request with only `turnId`.
- `docs/codex-app-server-harness.md` documents that Codex thread ids are stored
  in `.opensymphony/conversation.json` and that debug deeplinks use
  `codex://threads/<thread-id>`.
- [Installer And Distribution](../installer-and-distribution.md) keeps
  `cargo install opensymphony` turnkey, keeps raw Cargo out of native runtime
  distribution, keeps desktop/web clients as clients, and preserves
  `opensymphony run` as the execution-plane entrypoint.

## Goals

- Provide a real interrupt/cancel contract for both OpenHands agent-server and
  Codex app-server harnesses.
- Use that contract when the orchestrator observes that an active issue moved
  to `Merging` while its current turn is still in a `Human Review` polling loop.
- Wire the desktop `Cancel`, `Debug`, and `Workspace` buttons to concrete,
  user-visible behavior.
- Remove or hide Run Detail actions and panes that do not have real backing
  behavior.
- Bring desktop Run Detail and global status token rendering closer to the TUI.
- Add `opensymphony app` with visible alias `opensymphony desktop` as a lazy
  installer/launcher for desktop users without making default Cargo installs
  compile or bundle Tauri/npm/platform desktop dependencies.

## Non-Goals

- Do not make the desktop app the authority for scheduling state.
- Do not make `cargo install opensymphony` compile desktop dependencies by
  default.
- Do not replace `opensymphony run` as the local execution-plane entrypoint.
- Do not surface validation commands, validation evidence, or pending approvals
  in Run Detail unless the gateway exposes real data for those concepts.
- Do not add fake comments, follow-up issues, retries, or detach flows just to
  keep existing buttons visible.

## Harness Interrupt Contract

Add an orchestrator-owned cancel or interrupt command that is independent of any
specific harness protocol. The command should include:

- issue id and run id
- harness kind
- conversation id or thread id
- current turn id when the harness requires it
- reason, such as `operator_cancel` or
  `tracker_merging_supersedes_human_review`
- expected next state, such as `paused`, `interrupted`, `released`, or
  `closeout_pending`

OpenHands implementation requirements:

- Add an OpenHands client method for `POST /api/conversations/{id}/interrupt`
  or the equivalent configured base route.
- Treat `/interrupt` as the primary mechanism for mid-turn cancellation.
- Use `/pause` only as an explicit fallback for older agent-server versions that
  do not expose `/interrupt`, and record that fallback in diagnostics.
- Wait for the state/event reconciliation that proves the conversation reached
  paused or emitted an interrupt event before reporting acknowledgement.

Codex implementation requirements:

- Replace the stale `turn/cancel` JSON-RPC call with `turn/interrupt`.
- Send both `threadId` and `turnId`.
- Persist or retain the latest active Codex `turnId` on the run so the
  orchestrator can interrupt while the turn is in progress.
- Normalize `interrupted` turn status and any interrupt-related notification
  into the same run cancellation acknowledgement path used by OpenHands.

Gateway and state requirements:

- Surface `cancel_requested`, `cancel_acknowledged`, `cancel_failed`, and the
  cancel reason in run diagnostics.
- Make interrupt idempotent for repeated operator clicks or repeated tracker
  reconciliation events.
- Do not mark scheduler state terminal until the harness acknowledgement or
  configured timeout path has been recorded.

## Merging Supersedes Human Review Polling

When tracker polling observes an active issue transition to `Merging` while the
same issue conversation is still processing a `Human Review` polling turn:

1. Record a run event that the tracker state superseded the review polling turn.
2. Send the harness interrupt command with reason
   `tracker_merging_supersedes_human_review`.
3. Stop enqueueing new Human Review polling prompts for that issue.
4. After interrupt acknowledgement or timeout diagnostics, route the same issue
   through the existing landing or closeout workflow from `WORKFLOW.md`.
5. Reconcile Linear and PR state before deciding whether the issue should move
   to `Done`.

This is a state transition inside the same task run/conversation. It is not a
handoff between two workers.

## Desktop Run Detail Actions

`Cancel`:

- Dispatch the gateway cancel action for the selected run.
- Disable the button while cancellation is pending.
- Show acknowledgement, failure, or timeout in the Run Detail diagnostics.
- Avoid retrying or starting a new run as part of cancel.

`Debug`:

- For OpenHands runs, copy a shell-safe command to the clipboard:
  `cd <path-to-target-repo> && opensymphony debug <issue-key>`.
- Show a toast or tooltip that tells the operator the debug command is ready to
  paste in a terminal.
- For Codex runs, construct `codex://threads/<session-id>` from the recorded
  session/thread id.
- Prefer opening the Codex deeplink directly from the desktop shell.
- If the platform rejects the deeplink, copy it to the clipboard and show a
  toast that it can be pasted in a browser.

`Workspace`:

- Copy the local workspace path to the clipboard.
- Show a toast confirming the copied path.
- Keep any reveal-in-file-manager behavior as a separate future action if the
  product needs both copy and reveal.

Remove or hide these buttons from Run Detail until each has a real, tested API
or command:

- `Retry`
- `Detach`
- `Comment`
- `Follow-up`

`Rehydrate` may remain only if the gateway exposes a real rehydrate action with
clear operator semantics and tests. Otherwise hide it with the same rule.

## TUI Parity For Run Detail

Run Detail should replace current placeholders with data that exists today or
is straightforward to expose:

- Show `Turns` as the observed turn count only. If `max_turns` is still useful,
  expose it as a secondary label or tooltip, not as `current/max`.
- Remove the `Validation commands`, `Validation evidence`, and
  `Pending approvals` placeholders unless backed by actual gateway fields.
- Add branch and PR URL. PR URL should be a clickable hyperlink that opens in
  the browser.
- Add per-run token usage with input, cache, output, and total values.
- Update the global status pane to show input, cache, output, and total tokens,
  matching the TUI's breakdown.
- Split file change statistics into separate addition and deletion spans so
  additions render green and deletions render red in both the summary row and
  per-file rows.

The desktop can initially source branch and PR URL through the same workspace
inspection strategy the TUI uses, but the long-term contract should expose these
fields through Run Detail or a workspace detail endpoint so desktop and web
clients do not need to run ad hoc GitHub CLI commands.

## Lazy Desktop Launcher

Add `opensymphony app` with visible alias `opensymphony desktop`.

The command is a lazy desktop installer/launcher:

- It must not make default `cargo install opensymphony` compile Tauri, npm, or
  platform desktop dependencies.
- On first run, materialize or download a versioned desktop bundle into
  `~/.opensymphony/desktop/<version>/`.
- Verify the selected bundle for platform, architecture, version, and checksum
  before launching.
- Cache the bundle and launch the cached binary on later runs.
- Provide a clear repair path when the cached bundle is missing, corrupted, or
  for the wrong version.
- Keep GUI launch behavior in this app/desktop command flow while
  `opensymphony run` remains the local execution-plane entrypoint.
- Keep the desktop app a client that connects to or starts a local host profile;
  it must not become scheduler authority.

## Proposed Task Slices

- Harness interrupt contract: add orchestrator command, diagnostics, and adapter
  trait behavior.
- OpenHands interrupt adapter: implement and test the agent-server
  `/interrupt` request and reconciliation path.
- Codex interrupt adapter: replace `turn/cancel` with `turn/interrupt`, track
  active turn ids, and normalize interrupted status.
- Merging supersedes review polling: interrupt the current review polling turn
  and continue the same issue to landing/closeout.
- Desktop Run Detail actions: wire `Cancel`, `Debug`, and `Workspace`; remove
  unbacked buttons.
- Desktop Run Detail parity: branch, PR link, token breakdown, turn count, and
  colored file stats.
- Global desktop token status: input/cache/output/total display and tests.
- Lazy desktop launcher: `opensymphony app` and `opensymphony desktop` alias
  with versioned cache materialization.

## Acceptance Criteria

- Operator cancel interrupts an active OpenHands run through `/interrupt` and
  records acknowledgement or failure in Run Detail.
- Operator cancel interrupts an active Codex turn through `turn/interrupt` with
  both `threadId` and `turnId`.
- A task that moves from `Human Review` to `Merging` stops its review polling
  turn promptly and continues the same issue toward land/closeout.
- Run Detail no longer shows unbacked `Retry`, `Detach`, `Comment`, or
  `Follow-up` buttons.
- Run Detail debug behavior is harness-specific and either opens or copies the
  right command/deeplink.
- Run Detail workspace behavior copies the workspace path.
- Run Detail shows branch, clickable PR URL, per-run token breakdown, and turn
  count without the confusing fraction.
- Desktop global status shows input, cache, output, and total tokens.
- File additions are green and deletions are red.
- `opensymphony app` and `opensymphony desktop` launch the cached desktop bundle
  without adding desktop build dependencies to the default Cargo install path.

## Validation Plan

- Rust unit tests for OpenHands interrupt request construction and status
  reconciliation.
- Rust unit tests for Codex `turn/interrupt` request construction and
  interrupted-status normalization.
- Orchestrator tests for the `Human Review` to `Merging` supersede path,
  including idempotent repeated tracker observations.
- Gateway schema tests for new run diagnostics, branch/PR fields, and token
  breakdown compatibility.
- TypeScript tests for Run Detail action visibility, token rendering, turn
  count rendering, PR link rendering, and colored file stats.
- Desktop shell tests for clipboard copy and deeplink/open fallback behavior.
- CLI tests for `opensymphony app` and `opensymphony desktop` alias parsing and
  cache path selection.
