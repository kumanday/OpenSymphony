# Codex Thread Reuse, Archival, And Recovery Specification

Date: 2026-07-10

Status: proposed

## Reader And Outcome

This specification is for the engineer implementing Codex app-server thread
lifecycle support in OpenSymphony. After reading it, they should be able to
implement retry reuse first, then terminal archival and debug recovery, without
creating replacement or orphaned threads.

## Decision

Implement and ship the work in this order:

1. Reuse the existing Codex thread for every later run of the same issue.
2. Preserve the workspace manifest when an issue becomes terminal.
3. Archive the canonical thread when the issue is terminal.
4. Unarchive that thread before `opensymphony debug` or a reopened issue resumes
   it.

Do not ship terminal archival before thread reuse. Archiving only the latest
manifest thread while retries continue creating replacements would hide one
thread but leave every earlier retry thread detached from the manifest. Reuse
eliminates that source of orphaned threads. Terminal archival then operates on
one canonical thread per issue.

The two implementation slices may land in separate pull requests, but the
terminal archival slice must depend on and deploy after the reuse slice.

## Current Behavior And Root Causes

The current Codex worker starts a new app-server thread for every run:

1. initialize app-server
2. send `thread/start`
3. replace the conversation manifest with the returned thread id
4. send `turn/start`

This has four consequences:

- every retry creates another persisted Codex thread;
- the manifest retains only the newest thread id;
- earlier retry threads cannot be reached through `opensymphony debug`;
- a manifest write failure after `thread/start` can leave a new thread with no
  durable OpenSymphony reference.

The adapter's existing resume request is not a real thread resume. It labels a
request as `Resume` but sends `turn/start` directly. The installed Codex
app-server contract requires `thread/resume` before starting a new turn in a
saved thread.

Terminal cleanup has a second root cause. The runtime workspace backend removes
the workspace directory directly when the scheduler releases a terminal issue.
That bypasses the workspace manager's configured retention policy. The default
runtime policy says terminal workspaces are retained, but the direct removal
also deletes the conversation manifest needed to archive or debug the thread.

Finally, `opensymphony debug` currently invokes only:

```text
codex resume <thread-id>
```

Codex refuses to resume an archived thread until it is unarchived. The installed
CLI's archive and unarchive commands are also not idempotent: archiving an
already archived thread and unarchiving an active thread both fail. Lifecycle
code must inspect the actual archive state instead of issuing either operation
blindly.

## Goals

- Keep exactly one canonical Codex thread id per issue.
- Reuse that id across continuation, failure, stalled, cancelled, and
  reconciliation retries.
- Send the full workflow prompt once, then send continuation guidance.
- Never replace a valid canonical thread because resume failed.
- Archive the canonical thread after the issue becomes terminal.
- Retain enough workspace metadata for later debug recovery.
- Unarchive before interactive debug or before work resumes on a reopened issue.
- Make lifecycle transitions recoverable after process interruption.
- Keep old manifests readable without migration.

## Non-Goals

- Exporting or importing Codex conversations.
- Recreating a missing thread automatically.
- Adding a general conversation-history registry.
- Automatically rearchiving immediately after an interactive debug session.
- Changing OpenHands conversation reuse or archive behavior.
- Repairing the Codex desktop sidebar.
- Adding a configuration flag for canonical thread reuse.

## Invariants

The implementation must preserve these invariants:

1. A Codex-backed issue has at most one canonical thread id in its conversation
   manifest.
2. A retry never calls `thread/start` when a valid Codex manifest already
   exists.
3. A failed `thread/resume` never falls back to `thread/start`.
4. The canonical thread id does not change across retries, daemon restarts,
   terminal archival, debug unarchive, or issue reopen.
5. Only an explicit future reset or migration operation may replace the
   canonical id.
6. The full workflow prompt is recorded as seeded only after Codex accepts its
   `turn/start` request.
7. A terminal workspace is not removed before its canonical thread is archived.
8. Archive and unarchive decisions use Codex app-server state, not Codex's
   private SQLite schema.
9. Any unrecoverable lifecycle error fails closed: preserve the manifest and
   report the canonical thread id instead of creating a replacement.

## Durable Lifecycle State

Extend the existing conversation manifest with one backward-compatible field:

```text
codex_archive_state: active | archiving | archived | unarchiving
```

The field defaults to `active` when absent. It is meaningful only when the
manifest identifies the Codex app-server transport. Existing timestamps and
event fields record when the transition happened; no second archive timestamp
field is required.

The manifest remains the authority for which thread belongs to the issue. Codex
app-server remains the authority for whether that thread is actually active or
archived. Pending states allow a later process to reconcile a crash between the
Codex operation and the manifest update.

## Archive-State Inspection

Add a small Codex lifecycle query that finds the canonical thread id through
`thread/list`:

1. Query with the exact workspace CWD, `archived: true`, and
   `useStateDbOnly: true`; follow cursors until the id is found or exhausted.
2. If not found, repeat with `archived: false`.
3. Return `archived`, `active`, or `missing`.

Use the stable archived filter rather than inferring state from the thread's
rollout path. Include the normal top-level source kinds used by CLI, app-server,
and VS Code sessions. Do not include sub-agent-only source kinds.

This query is used only at lifecycle boundaries: worker startup with an existing
manifest, terminal transition, debug, and recovery of a pending transition. It
must not run on every runtime event.

## Slice One: Canonical Thread Reuse

### Manifest Decision Table

At Codex worker startup, load the conversation manifest before deciding which
thread request to send.

| Manifest state | Action |
| --- | --- |
| No manifest | Start one new thread and persist it. |
| Valid Codex manifest, thread active | Resume the recorded thread. |
| Valid Codex manifest, thread archived | Unarchive it, then resume it. |
| Valid Codex manifest, thread missing | Fail with repair guidance; do not start a replacement. |
| Invalid Codex manifest | Fail with the manifest path and parse/validation error. |
| Manifest belongs to another harness | Fail with an explicit harness-mismatch error. |
| Pending archive transition | Inspect Codex state, repair the manifest state, then continue. |

A manifest is reusable only when:

- the issue id and identifier match the current issue;
- `reuse_policy` is `per_issue`;
- the persistence directory matches the current workspace metadata directory;
- the transport and runtime contract identify Codex app-server;
- the thread id is non-empty.

An incompatible manifest is an operator-visible error, not permission to create
a replacement thread.

### First Run

For an issue with no conversation manifest:

1. Initialize app-server and send `thread/start` with the existing automation
   policy, current workspace CWD, current model, route metadata, and a persisted
   non-ephemeral thread.
2. Validate and capture the returned thread id.
3. Persist a manifest with that id, `fresh_conversation: true`,
   `workflow_prompt_seeded: false`, and `codex_archive_state: active`.
4. Send `turn/start` with the full workflow prompt.
5. After Codex accepts `turn/start`, update the same manifest with
   `workflow_prompt_seeded: true`, prompt kind `Full`, and the prompt/event
   timestamps.

If step 3 fails, archive the newly created thread through the same app-server
session as best-effort rollback, report its id, and fail the run. Do not continue
to `turn/start`.

If step 4 fails after the manifest was written, keep the manifest. The next run
resumes the same thread and retries the full prompt because
`workflow_prompt_seeded` is still false.

### Later Runs

For a valid existing manifest:

1. Inspect archive state and complete any pending transition.
2. If archived, unarchive the same thread and persist `active`.
3. Send `thread/resume` with the recorded thread id, current workspace CWD,
   current model override, and the existing automation policy.
4. Verify the response contains the same thread id.
5. Select the prompt:
   - use the full workflow prompt when `workflow_prompt_seeded` is false;
   - otherwise use the existing continuation guidance used by the OpenHands
     per-issue reuse path.
6. Send `turn/start` on the resumed thread.
7. Update attachment, prompt, and event metadata without replacing `created_at`
   or the canonical thread id.

Move continuation-guidance rendering to a shared harness-neutral helper so
OpenHands and Codex cannot drift into different retry instructions.

The current adapter method that labels a direct `turn/start` as resume must be
replaced with two distinct operations:

- a real `thread/resume` request;
- the existing `turn/start` request for continuation input.

### Resume Failure

If archive-state inspection, unarchive, `thread/resume`, response validation, or
`turn/start` fails:

- keep the existing manifest and canonical id;
- finish the worker as failed through the existing retry policy;
- include the issue identifier, thread id, lifecycle operation, and Codex error
  in structured diagnostics;
- do not call `thread/start` during that attempt.

This rule is the main protection against future orphaned threads.

## Slice Two: Terminal Archive And Debug Recovery

### Preserve The Manifest

Before adding terminal archival, change the runtime workspace backend to use the
workspace manager's cleanup decision and hooks. It must not remove the directory
directly.

The current runtime policy retains terminal workspaces. Retention keeps the
issue manifest, conversation manifest, run artifacts, and workspace path
available to `opensymphony debug`.

If terminal workspace removal becomes configurable in the future, successful
thread archival and durable storage of the canonical id must become a hard
precondition to removal. That removal mode is outside this implementation.

### Terminal Reconciliation

After a successful scheduler tick identifies terminal issues, and before the
next snapshot is published:

1. Find each retained terminal issue workspace.
2. Load its conversation manifest.
3. Skip missing manifests and non-Codex manifests.
4. Inspect the canonical thread's actual archive state.
5. If already archived, repair any stale manifest state to `archived`.
6. If active, persist `archiving`, send `thread/archive`, then persist
   `archived`.
7. If missing or the operation fails, retain the workspace, emit a structured
   warning, and retry on a later tracker tick.

Repeated terminal polls must be no-ops after both Codex and the manifest report
`archived`.

Do not use a one-shot in-memory completed set as the only guard. The durable
manifest state is required across daemon restarts.

### Pending-State Recovery

On startup or the next lifecycle boundary:

- `archiving` plus Codex archived becomes `archived`;
- `archiving` plus Codex active retries archive;
- `unarchiving` plus Codex active becomes `active`;
- `unarchiving` plus Codex archived retries unarchive;
- any pending state plus a missing Codex thread fails closed with repair
  guidance.

This makes process interruption between the external operation and manifest
write recoverable without parsing error strings or touching Codex's private
database.

### Debug

For a Codex-backed issue, `opensymphony debug` must:

1. Read the canonical id and archive state from the conversation manifest.
2. Inspect actual Codex archive state.
3. If archived, persist `unarchiving`, send `thread/unarchive`, and persist
   `active`.
4. Invoke `codex resume <thread-id>` in the issue workspace.
5. Preserve the same thread id regardless of the interactive command's exit
   status.

If unarchive fails, do not invoke resume. Print the exact thread id and the
manual recovery command.

Do not automatically rearchive when the interactive debug process exits. A
debug session may intentionally reactivate work, and one newly visible thread
does not recreate the original sidebar-pressure problem. If the issue remains
terminal, normal terminal reconciliation archives it again on the next run.

The `--app` deep-link path should perform the same unarchive step before printing
or opening the link; an archived thread link is otherwise not useful.

### Reopened Issues

When a terminal issue returns to an active state, worker startup follows the same
path as debug:

1. inspect the canonical thread;
2. unarchive it when necessary;
3. send `thread/resume`;
4. send continuation guidance in a new turn.

Reopening must not create a new thread.

## Implementation Surfaces

### Codex adapter

- Add typed request parameters for `thread/resume`, `thread/list`,
  `thread/archive`, and `thread/unarchive`.
- Add session request builders for those methods.
- Make the adapter's resume lifecycle emit `thread/resume`, not `turn/start`.
- Keep outbound request validation against the installed Codex schema.
- Reuse the existing thread-id response parser after renaming it to describe
  both start and resume responses.

### Conversation manifest

- Add `codex_archive_state` with a default of `active`.
- Preserve old manifests through Serde defaults.
- Add small mutation helpers for resumed, prompt-seeded, archiving, archived,
  unarchiving, and active transitions.
- Preserve the canonical id and original creation timestamp on every mutation.

### Codex worker

- Load and validate the manifest before choosing start or resume.
- Add the first-run and later-run state machine described above.
- Update the manifest after accepted lifecycle boundaries.
- Never use resume failure as an implicit reset signal.

### Runtime workspace backend

- Delegate terminal cleanup to the workspace manager.
- Honor the retained-workspace decision and lifecycle hooks.
- Remove the unconditional terminal directory deletion.

### Terminal reconciler

- Reuse the tracker snapshot's terminal issue set.
- Archive only Codex manifests that are not already durably archived.
- Retry failures on later ticks without blocking unrelated scheduler work.

### Debug command

- Use the shared archive-state inspection and unarchive operations.
- Update manifest state before launching interactive resume or emitting a deep
  link.
- Keep existing OpenHands debug behavior unchanged.

### Documentation

- Update the Codex app-server harness documentation to describe one thread per
  issue, real `thread/resume`, terminal archival, and debug unarchive.
- Document the manual recovery commands shown by lifecycle errors.

## Required Tests

### Adapter contract tests

- `thread/resume` serializes the installed schema's required `threadId` and
  current optional overrides.
- list, archive, and unarchive requests validate against the installed schema.
- resume response parsing accepts the real response shape and rejects a
  mismatched or empty id.

### Worker tests

- First run sends exactly one `thread/start`, persists its id, and sends a full
  prompt.
- A retry sends `thread/resume` for the same id and sends no `thread/start`.
- Multiple retry reasons all preserve the same id.
- An unseeded existing thread retries the full prompt.
- A seeded existing thread receives continuation guidance only.
- A current model override is included on resume without changing the id.
- Resume failure does not send `thread/start` and leaves the manifest unchanged.
- Manifest write failure after a new start attempts to archive the new thread
  and reports its id.

### Terminal tests

- Terminal cleanup honors retained workspace policy and does not delete the
  manifest.
- An active canonical thread is archived once when its issue becomes terminal.
- An already archived thread is a no-op.
- Archive failure leaves the workspace and manifest available for retry.
- `archiving` and `unarchiving` states reconcile correctly after simulated
  process interruption.
- A missing canonical thread fails closed and never starts a replacement.

### Debug tests

- Debug on an archived thread unarchives before resume.
- Debug on an active thread resumes without an archive mutation.
- Unarchive failure prevents resume and prints manual recovery guidance.
- The deep-link path unarchives before returning the link.
- OpenHands debug tests remain unchanged.

### End-To-End Regression

Run a fake app-server scenario with one issue through:

1. initial run;
2. successful continuation retry;
3. failed retry;
4. daemon restart;
5. terminal transition;
6. debug unarchive;
7. issue reopen and continuation.

Assert that every snapshot and manifest reports the same canonical thread id and
that only one Codex thread was ever started.

## Acceptance Criteria

- [ ] A three-attempt issue produces one Codex thread and three turns, not three
  threads.
- [ ] No retry or resume error path sends `thread/start` when a valid manifest
  exists.
- [ ] The full workflow prompt is sent once unless a prior full prompt was never
  accepted.
- [ ] Terminal transition archives the canonical thread and retains its
  manifest.
- [ ] Repeated terminal reconciliation and daemon restart do not duplicate or
  fail the archive operation.
- [ ] `opensymphony debug` can resume an archived terminal thread without manual
  unarchive.
- [ ] Reopening an issue resumes the same archived-then-unarchived thread.
- [ ] Existing OpenHands behavior and Codex interrupt behavior remain green.
- [ ] Installed-schema validation covers all newly used Codex methods.
- [ ] Structured errors always include the issue identifier and canonical
  thread id without including prompts or credentials.

## Rollout Plan

### Pull Request One: Reuse

- Implement real `thread/resume` and manifest-driven start/resume selection.
- Fix prompt seeding and continuation behavior.
- Add rollback for a newly started thread whose manifest cannot be persisted.
- Fix runtime workspace cleanup to honor retention.
- Ship all reuse and retention tests.

Before moving on, run one issue through at least three scheduler attempts and
verify that its manifest id never changes and Codex contains one thread for that
workspace.

### Pull Request Two: Archive And Recover

- Add archive-state inspection and durable transition state.
- Add terminal reconciliation archive.
- Add debug and reopened-issue unarchive.
- Add pending-transition recovery and lifecycle tests.
- Update operator documentation.

After deployment, run the existing superseded-session archiver in dry-run mode.
Its candidate count should remain zero while active and terminal issues continue
through scheduler cycles.

## Operational Recovery

Every lifecycle failure must print the canonical thread id. The operator can
inspect or repair it with:

```bash
codex unarchive <thread-id>
codex resume <thread-id>
```

After manual repair, rerunning `opensymphony run` reconciles the manifest's
pending state from Codex app-server. Manual recovery must never require editing
Codex's SQLite database.
