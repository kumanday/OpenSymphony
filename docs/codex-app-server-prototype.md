# Codex App-Server Prototype And Benchmark Report

COE-426 establishes a feature-gated Codex app-server integration shape. It does
not enable Codex as a production harness and does not route OpenSymphony issues
to Codex.

## Prototype Scope

- Feature gate: `codex-app-server-prototype`.
- Runtime kind: `codex_app_server`.
- Local transport: `codex app-server --stdio`.
- Experimental remote transport: `codex app-server --listen ws://127.0.0.1:<port>`.
- Contract source: generated Codex app-server JSON Schema and TypeScript
  bindings from the installed Codex CLI.

The prototype adds a small Rust module for:

- launch argument construction for stdio and loopback WebSocket,
- JSON-RPC request construction for `initialize`, `thread/start`, and
  `turn/start`,
- `thread/loaded/list` request use in the benchmark loop so throughput can be
  measured without starting model-backed turns,
- normalization of basic thread, turn, item, plan, error, and unknown
  notifications while preserving the raw payload,
- mapping existing OpenSymphony model and credential setting profiles to future
  Codex app-server use.

## Installed Codex Evidence

Captured on 2026-06-20 from this checkout:

```text
$ codex --version
codex-cli 0.138.0

$ codex app-server --help
Usage: codex app-server [OPTIONS] [COMMAND]
Commands: daemon, proxy, generate-ts, generate-json-schema
Options include --listen <URL>, --stdio, --ws-auth <MODE>,
--ws-token-file, --ws-token-sha256, --ws-shared-secret-file,
--ws-issuer, --ws-audience, and --ws-max-clock-skew-seconds.
```

A local stdio probe successfully started a JSON-RPC session:

```text
$ codex app-server --stdio
request: {"jsonrpc":"2.0","id":1,"method":"initialize",...}
response: {"id":1,"result":{"userAgent":"opensymphony-probe/0.138.0 ...",
"codexHome":"/home/user/.codex","platformFamily":"unix","platformOs":"macos"}}
```

Schema generation is supported:

```text
codex app-server generate-json-schema --out <dir>
codex app-server generate-ts --out <dir>
```

The generated protocol includes `initialize`, `thread/start`, `turn/start`,
`thread/started`, `turn/started`, `turn/completed`,
`item/agentMessage/delta`, `item/started`, `item/completed`, and server-side
approval request shapes.

## Benchmark Script

Run:

```bash
node scripts/codex_app_server_benchmark.mjs --iterations 50 --port 18765
```

The loopback WebSocket probe uses Node's global `WebSocket` and `fetch`
implementations and therefore requires Node.js 22 or newer. Use
`--skip-websocket` for stdio-only evidence on older Node runtimes.

The script performs:

- stdio `initialize` latency,
- loopback WebSocket readiness via `/readyz`,
- WebSocket `initialize` latency,
- queued `thread/loaded/list` request throughput and p50/p95 latency,
- reconnect by closing the socket, opening a new socket, and initializing again,
- secure exposure checks for localhost-only listener output and advertised
  capability-token/signed-bearer WebSocket auth flags.

Use `--skip-websocket` when the installed Codex version lacks WebSocket support.

## Local Benchmark Result

On this machine with `codex-cli 0.138.0`, stdio initialize and loopback
WebSocket probes are supported. A 10-request local run produced:

```json
{
  "generatedAt": "2026-06-20T06:34:40.884Z",
  "codexVersion": "codex-cli 0.138.0",
  "stdio": {
    "transport": "stdio",
    "initializeLatencyMs": 117.115,
    "response": {
      "id": 1,
      "result": {
        "userAgent": "opensymphony-codex-benchmark/0.138.0 (Mac OS 26.4.0; arm64) dumb (opensymphony-codex-benchmark; 0.0.0)",
        "codexHome": "/home/user/.codex",
        "platformFamily": "unix",
        "platformOs": "macos"
      }
    },
    "stderrBytes": 0
  },
  "websocket": {
    "transport": "websocket_loopback",
    "port": 18777,
    "initializeLatencyMs": 1.103,
    "queuedRequests": 10,
    "queuedResponses": 10,
    "queueElapsedMs": 0.905,
    "requestsPerSecond": 11049.22,
    "latencyMs": {
      "p50": 0.558,
      "p95": 0.67,
      "max": 0.67
    },
    "reconnectLatencyMs": 1.027,
    "reconnectResponse": "ok",
    "exposure": {
      "listener": "ws://127.0.0.1:18777",
      "localhostOnly": true,
      "authModesFromHelp": [
        "capability-token",
        "signed-bearer-token"
      ]
    }
  },
  "secureExposure": {
    "transport": "websocket_secure_exposure",
    "helpSha256": "ebddcbae81d5d6520609ad5605d069ddaf1d4c02cc97cc99d2585757aa4364ff",
    "hasCapabilityTokenMode": true,
    "hasSignedBearerMode": true,
    "hasTokenFileFlag": true,
    "hasSharedSecretFlag": true
  }
}
```

Loopback WebSocket starts with:

```text
codex app-server (WebSockets)
  listening on: ws://127.0.0.1:<port>
  readyz: http://127.0.0.1:<port>/readyz
  healthz: http://127.0.0.1:<port>/healthz
  note: binds localhost only (use SSH port-forwarding for remote access)
```

The production recommendation is to keep WebSocket feature-gated until CI or a
repeatable developer benchmark records stable throughput, queue, reconnect, and
auth behavior for the pinned Codex version.

## Model And Credential Reuse

Codex must reuse the gateway model settings shape instead of owning
subscription credentials. The current mapping is:

- `codex-chatgpt-local-keychain`: local subscription credential reference for
  future desktop/local Codex app-server use.
- `hosted-openai-subscription-broker`: hosted broker reference for future
  hosted Codex app-server or OpenHands subscription use.
- literal model references are converted into Codex config overrides where the
  app-server supports them.

Gaps:

- No production Codex credential reader is implemented in this issue.
- No raw subscription token is stored in an OpenSymphony workspace or sent to
  browser clients.
- Hosted credential broker support remains a follow-up implementation.

## Readiness Recommendation

Codex app-server is suitable for a feature-gated local prototype and contract
test path. Production enablement should wait for:

- a pinned Codex app-server protocol version and generated schema artifact
  policy,
- replay/history semantics for reconnect beyond one-shot request recovery,
- approval request mapping into OpenSymphony approval DTOs,
- subscription credential adapter completion,
- security review of non-loopback WebSocket exposure with capability-token and
  signed-bearer modes.
