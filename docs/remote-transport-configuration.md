# Remote Transport Configuration Notes

> COE-407 / OSYM-741 — Browser Transport And Remote Stream Protocols
> Milestone M10: Web Client And External Gateway

This documents the transport configuration boundaries for the local
orchestrator, external gateway, and hosted gateway profiles, plus origin/CORS
preparation for separate deployment. The browser transport is also the desktop
remote/hosted profile baseline.

## How profile selection works

A `ConnectionProfile` contributes the base URL, preferred `TransportProfile`,
auth token, and probe-on-connect behavior. Advertised `GatewayCapabilities`
decide which optional features (binary WebSocket frames, SSE vs WebSocket) are
enabled. There are **no client-side protocol forks**: every profile resolves to
one of the shared transport implementations (`HttpGatewayTransport`,
`WebSocketTransport`, `TauriChannelTransport`) through `TransportFactory`, and
`createTransportForProfile(profile, { authToken, capabilities })` wires the
profile + capabilities into a single `GatewayTransportConfig`.

```ts
const transport = await createTransportForProfile(profile, {
  authToken,
  capabilities,
});
```

`GatewayTransportConfig` now carries an optional `capabilities` field so
transports can opt into advertised features without per-profile branching.

## Profile boundaries

### Local orchestrator (local daemon / supervised / embedded)

- **Profiles**: `local_daemon`, `supervised_local_daemon`, `embedded_host`.
- **Base URL**: loopback, default `http://127.0.0.1:2468`.
- **Transport**: `loopback_http` or `loopback_websocket`, chosen from
  capabilities. Loopback origin is trusted, so CORS is not a factor.
- **Auth**: typically `none` on loopback; `api_key`/`bearer_token` when the
  daemon is shared. The same auth header is applied to both HTTP and stream
  channels (`Authorization: Bearer <token>` on HTTP; an `auth` message on
  WebSocket open).
- **Lifecycle**: `supervised_local_daemon` and `embedded_host` are `managed: true`
  (desktop owns the process). `local_daemon` is `managed: false`.
- **Binary frames**: available when the loopback gateway advertises binary
  support; useful for terminal/log streams even on loopback.

### External gateway

- **Profile**: `external_gateway` (`managed: false`, `probeOnConnect`).
- **Base URL**: user-configurable loopback or trusted-network URL.
- **Transport**: `loopback_http` / `loopback_websocket` / `sse` from
  capabilities. No daemon management by the client.
- **Auth**: `bearer_token` / `api_key`. The auth header is applied consistently
  to HTTP reads/mutations and to the WebSocket `auth` handshake so stream and
  control auth behavior match.
- **CORS**: when the web client and gateway are on different origins (the
  expected case for separate deployment), the gateway must send appropriate
  CORS headers for HTTP and the WebSocket handshake. See "Origin and CORS"
  below.

### Hosted gateway

- **Profile**: `hosted_gateway` (`managed: false`, `probeOnConnect`).
- **Base URL**: remote hosted server (HTTPS/WSS).
- **Transport**: `websocket` / `sse` / `json_rpc_over_websocket` from
  capabilities. HTTPS for reads/mutations; WSS for streams.
- **Auth**: `bearer_token` or `subscription_oauth`. Auth is required on every
  HTTP request and on the WSS handshake + per-message (defense in depth).
- **RBAC**: hosted RBAC middleware is out of scope for this ticket but must be
  enforceable per request/method on the hosted gateway.
- **Consistency**: hosted consistency takes priority over raw throughput. The
  client enforces cursor replay, idempotency, action receipts, and monotonic
  sequences regardless of the hosted transport selected.

## Consistent HTTP and stream auth behavior

All profiles apply auth identically across channels:

- **HTTP** (reads, mutations, capability discovery): `Authorization: Bearer
  <token>` header via `GatewayTransportConfig.authToken`.
- **WebSocket**: on open, the client sends an `auth` message
  (`{ type: "auth", token }`) in addition to any transport-layer credentials.
  This keeps stream auth behavior consistent with HTTP auth and works in
  browsers where custom headers on the WebSocket handshake are limited.
- **SSE**: auth via the same `Authorization` header on the EventSource request
  (or a token query param when the browser EventSource API cannot set headers).

## Origin and CORS preparation (separate deployment)

When the web client is deployed separately from the gateway (different origin),
prepare the gateway so the browser can reach it:

- **HTTP reads/mutations**: gateway must respond with
  `Access-Control-Allow-Origin` for the web client origin (or a controlled
  allowlist), and handle preflight (`OPTIONS`) for mutating methods. Credentials
  (`Authorization`) require `Access-Control-Allow-Credentials: true` and a
  non-wildcard origin.
  - **Origin allowlist source**: the gateway obtains the allowed origin list
    from its own configuration (an explicit `allowed_origins` allowlist keyed to
    the deployment profile), never from the request. Operators configure it per
    profile (local orchestrator, external gateway, hosted gateway) before
    separate-deployment production use.
  - **Wildcard-with-credentials failure**: if the gateway misconfiguration
    returns `Access-Control-Allow-Origin: *` together with
    `Access-Control-Allow-Credentials: true`, browsers reject the credentialed
    response (the fetch fails with a CORS error and no body is exposed). The
    client surfaces this as a standard fetch failure: capability discovery /
    reads return a rejected promise (transport error) and mutations fail before
    the action is accepted. Operators must resolve the misconfiguration by
    replacing `*` with the explicit web client origin; the client does not retry
    past a CORS rejection beyond the normal reconnect/backoff policy.
- **WebSocket**: the browser enforces origin checks on the WS handshake. The
  gateway should validate the `Origin` header against an allowlist and reject
  cross-origin WS connections that are not expected. Unlike HTTP, WS does not
  use CORS response headers; origin validation is server-side.
- **SSE**: subject to the same CORS rules as HTTP `GET`.
- **Configuration boundary**: origin allowlists and CORS settings are gateway
  configuration, not client configuration. The client never forks behavior by
  origin; it just points at the configured `gatewayUrl`. This keeps the client
  deployment-agnostic.
- **Preparation status**: this ticket prepares the client side (no origin forks,
  consistent auth). Gateway-side CORS/origin enforcement is tracked separately
  and must be configured before separate-deployment production use.

## Transport capability advertisement

The client reads `GET /api/v1/capabilities` (`GatewayCapabilities`) to learn:

- `transports[]`: each entry has `transport` (a `TransportProfile`), `modes`
  (e.g. `["json", "binary"]`), `supported_encodings` (e.g.
  `["utf-8", "binary", "base64"]`), and `bidirectional`.
- `auth_modes[]`: which auth modes the gateway accepts.
- `max_event_page_size`, `max_terminal_frame_batch`: pagination/batching limits.

`binaryFramesAdvertised(capabilities)` returns true when a WebSocket transport
advertises `binary` in `modes` or `supported_encodings`. The client enables
binary frames only then.

## Out of scope (tracked separately)

- Hosted RBAC middleware (tracked in
  [COE-472](https://linear.app/trilogy-ai-coe/issue/COE-472/hosted-gateway-rbac-enforcement-per-requestmethod)).
- Desktop local native transport (in-process/native IPC).
- Final production selection of Codex app-server WebSocket behavior.
- Gateway-side CORS/origin enforcement configuration.