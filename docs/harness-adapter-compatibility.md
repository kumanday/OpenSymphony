# Harness Adapter Compatibility

This memo records the COE-408 adapter fit notes for the shared harness capability
model. The public discovery surface is `HarnessCapability` in
`opensymphony-gateway-schema`; the Rust boundary is `HarnessAdapter` in
`opensymphony-domain`.

## Shared Contract

The capability DTO covers the minimum cross-harness questions clients and host
code need before selecting or rendering a harness:

- actions: start, message, retry, cancel, pause/resume, approvals, comments
- event streams: runtime events, terminal frames, cursor replay, raw payload refs
- approvals: tool approval, human decisions, policy metadata
- model settings: API-compatible settings, subscription credentials, per-run
  overrides, credential reference kinds
- transport: protocol, modes, local/remote support
- cancellation and pause/resume semantics
- history: REST/history fetch, readiness reconciliation, reconnect/replay,
  unknown-event preservation

## OpenHands Agent Server

OpenHands is the initial production adapter and fits the finalized shape through
the existing `IssueSessionRunner`. It supports conversation creation, message
send, run triggering, HTTP history fetch, WebSocket runtime events, readiness
reconciliation, reconnect/replay, unknown raw event preservation, retries,
cancellation, and terminal frames.

Known gaps:

- Pause/resume is not exposed by the current OpenHands agent-server contract.
- Approval-center normalization is not yet available through the gateway DTOs.
- Subscription credentials are separate future work; API-compatible env-backed
  model settings are the current supported execution path.
- The model settings endpoint can represent isolated OpenHands auth-directory
  references for ChatGPT/Codex subscription credentials, but the adapter that
  consumes those references is follow-up work.

## Codex App Server

Codex app-server fits the same contract as a future JSON-RPC adapter. Requests
map to start thread/turn, send input, approve/reject, and cancel operations.
Notifications map to OpenSymphony runtime events with raw payload retention,
cursor replay, and correlation IDs at the gateway layer.

COE-426 adds a feature-gated local prototype under
`codex-app-server-prototype`. The prototype can construct stdio launch
arguments, build `initialize`, `thread/start`, and `turn/start` JSON-RPC
requests, normalize basic thread/turn/item notifications, and map the existing
model credential settings profiles to future Codex use. Production routing
remains disabled; `codex_app_server` is still advertised as unavailable through
capability discovery.

Codex ChatGPT subscription readiness is now advertised as a Codex CLI login
reference rather than an OpenSymphony-owned secret. Capability discovery lists
`codex_cli_login` alongside the existing `inherited_subscription_login` as a
supported credential reference kind for the future Codex adapter. The gateway
model-settings response probes `codex --version`,
`codex app-server --help`, and `codex login status` to render installed,
logged-out, expired, unsupported, permission-denied, or unknown states without
reading private Codex credential files.

Benchmark and contract evidence lives in
`docs/codex-app-server-prototype.md`. The companion script
`scripts/codex_app_server_benchmark.mjs` exercises stdio initialize, loopback
WebSocket throughput, queued request behavior, reconnect, and secure exposure
flags against the installed Codex CLI.

Known gaps:

- Production adapter implementation is out of scope for COE-426.
- Pause/resume semantics need protocol confirmation before being advertised as
  available.
- WebSocket transport remains experimental until benchmarked and secured; stdio
  is the preferred local integration mode.
- Codex subscription readiness uses the local Codex CLI login reference and can
  later compose with hosted broker references without requiring raw subscription
  tokens in OpenSymphony workspaces or browser payloads.
- Production routing still needs a dedicated adapter that consumes the
  `codex_cli_login` reference only through supported Codex CLI/app-server
  behavior.

## Rust-Native Harness

A Rust-native or in-process harness fits the same contract by implementing
`HarnessAdapter` and normalizing its own run, event, approval, cancellation,
history, and evidence behavior into the gateway DTOs. The capability model allows
both in-process and subprocess/RPC modes.

Known gaps:

- Concrete SDK/runtime selection is not implemented yet.
- Hosted execution would need an isolation model before remote support is
  advertised.
