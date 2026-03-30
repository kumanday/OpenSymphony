---
name: debug
description:
  Investigate stuck runs and execution failures by tracing OpenSymphony logs
  with issue/session identifiers; use when runs stall, retry repeatedly, or
  fail unexpectedly.
---

# Debug

## Goals

- Find why a run is stuck, retrying, or failing.
- Correlate Linear issue identity to an OpenHands session quickly.
- Read the right logs in the right order to isolate root cause.

## Log Sources

- Primary runtime log: `log/symphony.log`
  - Includes orchestrator, agent runner, and OpenHands runtime lifecycle logs.
- Rotated runtime logs: `log/symphony.log*`
  - Check these when the relevant run is older.

## Correlation Keys

- `issue_identifier`: human ticket key (example: `MT-625`)
- `issue_id`: Linear UUID (stable internal ID)
- `conversation_id`: OpenHands conversation UUID
- `session_id`: OpenHands thread-turn pair (`<thread_id>-<turn_id>`)

Use these fields as join keys during debugging.

## Quick Triage (Stuck Run)

1. Confirm scheduler/worker symptoms for the ticket.
2. Find recent lines for the ticket (`issue_identifier` first).
3. Extract `conversation_id` or `session_id` from matching lines.
4. Trace that ID across start, stream, completion/failure, and stall handling logs.
5. Decide class of failure: timeout/stall, runtime startup failure, turn
   failure, or orchestrator retry loop.

## Commands

```bash
# 1) Narrow by ticket key (fastest entry point)
rg -n "issue_identifier=MT-625" log/symphony.log*

# 2) If needed, narrow by Linear UUID
rg -n "issue_id=<linear-uuid>" log/symphony.log*

# 3) Pull conversation IDs seen for that ticket
rg -o "conversation_id=[^ ;]+" log/symphony.log* | sort -u

# 4) Trace one conversation end-to-end
rg -n "conversation_id=<uuid>" log/symphony.log*

# 5) Focus on stuck/retry signals
rg -n "Issue stalled|scheduling retry|turn_timeout|turn_failed|session failed|session ended with error" log/symphony.log*
```

## Investigation Flow

1. Locate the ticket slice:
    - Search by `issue_identifier=<KEY>`.
    - If noise is high, add `issue_id=<UUID>`.
2. Establish timeline:
    - Identify first session start for the issue.
    - Follow with completion, error, or worker exit lines.
3. Classify the problem:
    - Stall loop: `Issue stalled ... restarting with backoff`.
    - Runtime startup failure: session creation/attach failures.
    - Turn execution failure: `turn_failed`, `turn_cancelled`, `turn_timeout`.
    - Worker crash: `Agent task exited ... reason=...`.
4. Validate scope:
    - Check whether failures are isolated to one issue/session or repeating across
      multiple tickets.
5. Capture evidence:
    - Save key log lines with timestamps, `issue_identifier`, `issue_id`, and
      `conversation_id`.
    - Record probable root cause and the exact failing stage.

## Reading Session Logs

OpenSymphony session diagnostics are emitted into `log/symphony.log` and
keyed by `conversation_id`. Read them as a lifecycle:

1. Session start: `conversation_id=...` created/attached
2. Session stream/lifecycle events for the same `conversation_id`
3. Terminal event:
    - Session completed successfully, or
    - Session ended with error, or
    - Issue stalled ... restarting with backoff

For one specific session investigation, keep the trace narrow:

1. Capture one `conversation_id` for the ticket.
2. Build a timestamped slice for only that conversation:
    - `rg -n "conversation_id=<uuid>" log/symphony.log*`
3. Mark the exact failing stage:
    - Startup failure before stream events (connection/attach errors).
    - Turn/runtime failure after stream events (`turn_*` / error events).
    - Stall recovery (`Issue stalled ... restarting with backoff`).
4. Pair findings with `issue_identifier` and `issue_id` from nearby lines to
   confirm you are not mixing concurrent retries.

Always pair session findings with `issue_identifier`/`issue_id` to avoid mixing
concurrent runs.

## Notes

- Prefer `rg` over `grep` for speed on large logs.
- Check rotated logs (`log/symphony.log*`) before concluding data is missing.
- If required context fields are missing in new log statements, align with
  logging conventions.