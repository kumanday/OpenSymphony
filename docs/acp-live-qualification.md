# ACP v1 live qualification

This report qualifies the local stdio ACP route on 2026-09-25 with two
independently implemented CLIs. The tests use disposable issue workspaces and a
local Linear GraphQL fixture, but launch the real `opensymphony run` command,
production routing and worker, gateway, and vendor ACP processes. The OpenHands
URL is deliberately unreachable in each edit scenario; a successful ACP run
therefore also checks that the route does not start or fall through to OpenHands.
No credentials, account identifiers, opaque session IDs, or unredacted source
frames are included in this report.

| Profile | Pinned CLI and auth | ACP negotiation observed | Live result and boundary |
| --- | --- | --- | --- |
| Cursor | `agent` `2026.09.08-6caf4ff`; existing Cursor login, advertised `cursor_login` authentication | v1 stdio JSON-RPC; `loadSession: true`; no `resumeSession` advertisement; session modes and two config options | Tracked edit, gateway plan rejection, cancellation acknowledgement, `session/load` restoration, and registered `cursor/create_plan`/`cursor/update_todos` request-response paths passed. Only these observed extension methods are enabled for this pin. |
| Devin | `devin` `3000.10.21 (611c1cba)`; `devin auth status` reported logged in via Devin | v1 stdio JSON-RPC; `loadSession: true`; no `resumeSession` advertisement; session modes and three config options | Tracked edit and `session/load` restoration passed. The CLI sent `config_option_update`, `current_mode_update`, and `available_commands_update` for the new session before the `session/new` response. The response is authoritative for fields it supplies; earlier updates fill only omitted fields. No Devin extension is registered. |

The two implementations were launched from separate vendor executables with
separate authentication and session implementations. Generic ACP v1 facilities
are negotiated per session; a configured profile does not imply that a model,
mode, MCP transport, or optional persistence method is available. Both live
CLIs advertised load and restored a known-finished, retired session with the
same bound session ID. The second process sent `session/load` before any new
`session/prompt`; a fresh second prompt then completed. Neither CLI advertised
`session/resume`, so no live resume-without-replay claim is made. The fake-peer
suite separately covers load replay, resume without replay, and the no-persistence
fresh-session path. Replayed source frames are tagged and excluded from live
usage accounting; a possibly submitted prompt is never replayed on recovery.

## Reproduce

Verify the CLI versions and authenticated status locally, then set
`OPENSYMPHONY_CURSOR_AGENT_BIN` and `OPENSYMPHONY_DEVIN_BIN` to the respective
executables. The manual tests are ignored by unauthenticated CI. On a macOS
installation with system DuckDB 1.5.3, run:

```sh
agent --version
devin --version
devin auth status
cargo test-system-duckdb --test run live_cursor_acp_tracked_issue_edits_workspace -- --ignored --nocapture
cargo test-system-duckdb --test run live_devin_acp_tracked_issue_edits_workspace -- --ignored --nocapture
cargo test-system-duckdb --test run live_cursor_acp_tracked_issue_returns_operator_plan_decision -- --ignored --nocapture
cargo test-system-duckdb --test run live_cursor_acp_tracked_issue_cancel_acknowledgement -- --ignored --nocapture
cargo test-system-duckdb --test acp_session_host pinned_live_acp_session_load_restores_without_prompt_resend -- --ignored --nocapture
cargo test-system-duckdb --test acp_session_host pinned_cursor_live -- --ignored --nocapture
```

The first two `run` tests assert a succeeded tracked attempt, exact
`acp-live-proof.txt` workspace content, and the `acp` route/profile manifest.
The operator test rejects a wrong-session reply, then returns a plan rejection
through the gateway to the live Cursor callback. The cancellation test waits
for a submitted prompt, dispatches a gateway cancel action, then requires the
scheduler's `interrupt_acknowledged` event with `ACP prompt stopped: cancelled`.
The restoration test asserts a `session/load` source method, unchanged bound
session, no replayed prompt, and a successful subsequent prompt. The pinned
Cursor host tests assert the redacted source-frame method, ID-bearing request,
absence of a supplied `sessionId`, and the matching result shape for both
supported extension methods. See
[extension evidence](acp-extension-evidence.md) for their bounded schemas.

## Conformance and failure matrix

| Contract | Executable evidence | Qualification |
| --- | --- | --- |
| New-session update ordering | `tests/acp.rs` pre-response update, authoritative response, and foreign-session cases | Devin-compatible setup retains omitted fields and rejects a mismatched session before prompt submission. A bounded queue applies before the response. |
| Cross-session operator and callback safety | `tests/acp.rs`, `tests/acp_session_host.rs`, and live tracked Cursor plan test | Wrong-session replies and updates cannot reach another session. An accepted operator receipt requires callback delivery. |
| Cancellation and cleanup | Live tracked Cursor cancel; ACP host and worker cancellation tests | Acknowledgement follows the matching stopped prompt response. Unknown/uncertain submission fences cleanup and retry; owner retirement requires quiescence. |
| Continuation and restart | `tests/acp_session_host.rs` load/resume/none recovery and live Cursor/Devin load test; `crates/opensymphony-cli` worker recovery tests | Known-finished context restores only through advertised methods. No ambiguous prompt resend; absent persistence starts fresh with the full prompt. |
| Replay and usage | ACP host restoration and worker usage tests | Load history is tagged as replay and does not double-count usage; resume has no replay. |
| Model/config and credential variation | `tests/acp.rs` model/mode/options, environment-reference, authentication and redaction cases | The generic client validates configured choices and fails before prompt on unsupported selections or authentication failure. No cross-vendor live model equivalence is claimed. |
| Discovery and schema parity | Gateway schema round-trip, capabilities endpoint, adapter-boundary, and TypeScript checks | Public adapter support is distinct from profile preflight and run-negotiated capability. Preflight does not attest authentication. |

The generic profile needs only `routing.harness: acp`, a selected
`routing.harness_profile`, and an `acp.profiles` stdio command/args entry. It
does not require a vendor-specific scheduler branch. Prefer `env_refs` for
credentials; never put secret values into the command, arguments, workflow, or
this evidence report. The process runs with the issue workspace as its cwd and
inherits trusted local host access, so choose profiles and permission policy
accordingly. Operator decisions require a live gateway response path.
