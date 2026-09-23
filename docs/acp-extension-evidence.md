# ACP extension qualification evidence

## Pinned Cursor CLI probe

Probe date: 2026-09-23 UTC. Executable:
`/Users/magos/.local/bin/agent`, Cursor CLI `2026.09.08-6caf4ff`, invoked as
`agent acp` from the isolated COE-613 checkout. The JSON-RPC request IDs and
response fields below are real frames, with the local `cwd` and unrelated
client identity omitted. No credentials or account identifiers were captured.

```json
{"id":0,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}}}
{"id":0,"result":{"protocolVersion":1,"authMethods":[{"id":"cursor_login"}],"agentCapabilities":{"loadSession":true,"mcpCapabilities":{"http":true,"sse":true},"promptCapabilities":{"image":true},"sessionCapabilities":{"list":{}}}}}
{"id":1,"method":"session/new","params":{"cwd":"<isolated issue checkout>","mcpServers":[]}}
{"id":1,"error":{"code":-32000,"message":"Authentication required","data":{"message":"Authentication required. Please run 'agent login' first, then call authenticate() with methodId 'cursor_login'."}}}
```

`agent status` reported `Not logged in`. The probe reached initialization but
could not create a session or observe a Cursor extension callback. A live
`cursor/ask_question`, `cursor/create_plan`, or notification frame and its
response remain required before vendor compatibility is qualified.

## Deterministic contract evidence

`tests/fixtures/acp_session_peer.py` simulates documented Cursor callback
envelopes, including a malformed request, request ID `0`, a todo notification,
and an unknown request. `tests/acp_session_host.rs` verifies operator routing,
the documented question result shape, and that notifications receive no RPC
response. The fixture echo peer advertises an explicit capability and verifies
the owner-supplied session ID, permitted `_meta`, structured result, invalid
arguments, and a timeout that reports outcome unknown without retry. These
fixtures establish OpenSymphony behavior; they do not substitute for the live
Cursor callback qualification above.
