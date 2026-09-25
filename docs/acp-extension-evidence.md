# ACP extension qualification evidence

## Pinned Cursor CLI

The authenticated Cursor CLI `2026.09.08-6caf4ff` was invoked as `agent acp`
from disposable issue workspaces on 2026-09-25 UTC. Initialization negotiated
ACP protocol version 1; `authenticate` with the advertised `cursor_login` method
and `session/new` succeeded. The probes requested chat-only color answers and
disabled client filesystem and terminal capabilities. No credentials, account
identifiers, absolute workspace paths, or raw session IDs are retained here.

Plan mode emitted an ID-bearing `cursor/create_plan` **request**. It had no
`params.sessionId`; the active ACP session supplied the binding. Its observed
envelope and the accepted and rejected result shapes are:

```json
{"jsonrpc":"2.0","id":0,"method":"cursor/create_plan","params":{"toolCallId":"<tool-call-id>","name":"Chat Color Answer","overview":"<chat-only overview>","plan":"<bounded plan text>","todos":[{"id":"chat-answer","content":"<chat-only todo>","status":"pending"}],"isProject":false,"phases":[]}}
{"jsonrpc":"2.0","id":0,"result":{"outcome":{"outcome":"accepted"}}}
{"jsonrpc":"2.0","id":0,"result":{"outcome":{"outcome":"rejected"}}}
```

The two result lines represent separate safe sessions, not duplicate responses
to one request. Both results let the plan prompt finish normally. In agent mode
the CLI emitted `cursor/update_todos` with numeric
ID `0` and no `params.sessionId`. The host must answer this request even though
Cursor's public documentation describes todo updates as notifications. The
observed envelope and accepted response are:

```json
{"jsonrpc":"2.0","id":0,"method":"cursor/update_todos","params":{"toolCallId":"<tool-call-id>","todos":[{"id":"color-probe","content":"Answer the color probe","status":"completed"}],"merge":false}}
{"jsonrpc":"2.0","id":0,"result":{"outcome":{"outcome":"accepted","todos":[{"id":"color-probe","content":"Answer the color probe","status":"completed"}]}}}
{"jsonrpc":"2.0","id":0,"result":{"outcome":{"outcome":"rejected"}}}
```

The accepted and rejected todo results came from separate safe sessions; both
ended with `stopReason: end_turn`. The pinned CLI completed the todo prompt even
when a separate probe delayed its response four seconds. The JSON-RPC ID still makes the frame a request, so the
production host validates and responds. The runtime owner, rather than a
client-supplied session field, binds both methods to the active ACP session.
Unqualified `cursor/ask_question`, `cursor/task`, and `cursor/generate_image`
methods are not enabled by this registration. A no-ID Cursor notification was
not observed; deterministic fixtures cannot establish its pinned live behavior.

## Production path and deterministic regressions

The manual authenticated integration tests run the actual pinned CLI through
`SessionHost`, not the standalone probe. The plan test received the real ID `0`
request without a session field, opened a `PlanApproval` interaction bound to
the host-owned session, delivered an operator rejection, observed the exact
rejected result, and completed the prompt. The todo test received the real ID
`0` request without a session field, sent the accepted result with the todo
list, and completed the prompt. Invalid or unbound todo requests receive the
observed rejected result shape. Scheduler todo activity is projected only from
the correlated accepted response; a rejected request creates no todo update.
Duplicate in-flight IDs remain ambiguous until every matching response drains.
At 16 unresolved todo requests, another request emits a bounded saturation
diagnostic and fences the worker rather than silently dropping accepted activity.
Both use disposable issue workspaces, are
ignored in unauthenticated CI, and passed when invoked manually:

```text
OPENSYMPHONY_CURSOR_AGENT_BIN=/path/to/pinned/agent cargo test-system-duckdb --test acp_session_host pinned_cursor_live -- --ignored --nocapture
  2 passed; 0 failed; finished in 12.19s
```

`tests/fixtures/acp_session_peer.py` gives repeatable plan and todo ID `0`
callbacks without `sessionId`, plus a mismatched-session plan and unknown
method. A simulated no-ID frame verifies the generic no-response rule without
claiming live Cursor notification support. The host fixture verifies operator
routing, plan acceptance, todo response, and safe rejection. The fixture echo peer advertises an explicit
capability and verifies the owner-supplied session ID, permitted `_meta`,
structured result, invalid arguments, and a timeout that reports outcome
unknown without retry. A 24-trial regression verifies that prompt completion
does not discard a correlated operation response. A synthetic credential in
the result value and metadata is redacted before the public operation result;
successive timed-out batches remain inside the eight unresolved-request limit.
