# Codex App-Server Local Harness And Benchmark Report

COE-426 established the benchmark/prototype integration shape. COE-476 promotes
the local stdio path into a supported OpenSymphony harness capability while
leaving hosted worker pools and loopback WebSocket routing out of production
scope.

## Local Harness Scope

- Runtime kind: `codex_app_server`.
- Supported local transport:
  `codex --dangerously-bypass-hook-trust app-server --stdio`.
- Experimental loopback WebSocket transport:
  `codex --dangerously-bypass-hook-trust app-server --listen ws://127.0.0.1:<port>`.
- Contract source: generated Codex app-server JSON Schema and TypeScript
  bindings from the installed Codex CLI.

The Rust module provides:

- launch argument construction for stdio and loopback WebSocket,
- JSON-RPC request construction for `initialize`, `thread/start`, and
  `turn/start`, plus resume, cancel, and approval responses,
- normalization of thread, turn, item, approval, cancellation, error, and
  unknown notifications while preserving the raw payload,
- mapping existing OpenSymphony model and credential setting profiles to Codex
  app-server use,
- a concrete `HarnessAdapter` implementation for the local stdio capability.

The current request builder defaults `modelProvider` to `openai` because Codex
CLI app-server exposes local OpenAI/ChatGPT-backed model ids on this path. If a
future Codex CLI adds provider-neutral model routing, OpenSymphony should pass
that provider through the harness adapter instead of treating it as a local
default.

The companion benchmark script issues `thread/loaded/list` requests directly so
throughput can be measured without starting model-backed turns.

## Full-Automation Profile

OpenSymphony targets a trusted local automation profile for Codex app-server.
The server process is launched with hook trust bypass as a Codex CLI argument;
this is not prompt text and not a `turn/start` JSON field:

```bash
codex --dangerously-bypass-hook-trust app-server --stdio
```

The run then creates the Codex thread with the selected working directory,
selected model when present, `approvalPolicy: "never"`, and the installed
schema's thread sandbox value `sandbox: "danger-full-access"`. OpenSymphony
starts the actual task through `turn/start` with the rendered workflow prompt as
input plus the maximum-permission turn profile:

```json
{
  "approvalPolicy": "never",
  "sandboxPolicy": {
    "type": "dangerFullAccess"
  }
}
```

`approvalPolicy: "never"` means OpenSymphony does not wait for human approval
callbacks from Codex. Execution failures are streamed back through Codex and
handled by the model loop. `dangerFullAccess` intentionally carries no
`networkAccess` field; effective network access comes from the host/container
environment because Codex is not applying its normal sandbox boundary.

This profile is only appropriate when OpenSymphony itself is running Codex
inside an externally isolated environment such as a disposable workspace,
container, VM, or equivalent trusted local runner.

## Local Testing

Codex app-server local stdio support is compiled into normal OpenSymphony
builds. The old `codex-app-server-prototype` Cargo feature has been removed;
adapter contract and benchmark tests run through the normal local harness
module.

Use the system DuckDB developer aliases for quick local verification:

```bash
cargo check-system-duckdb
cargo test-system-duckdb --test codex_app_server
```

Install or select the Codex CLI that should be tested, then confirm the
app-server surface exists:

```bash
codex --version
codex app-server --help
```

Confirm the local Codex CLI is signed in with ChatGPT:

```bash
codex login status
```

If the CLI is not logged in, use the current Codex-supported device-code auth
path:

```bash
codex login --device-auth
```

For ChatGPT accounts that have not previously allowed Codex device-code sign-in,
open ChatGPT settings and enable **Security and login -> Enable device code
authorization for Codex** before retrying the login:

![ChatGPT setting for enabling Codex device-code authorization](images/enable-device-code-authorization-for-codex.png)

After login, run a tiny real Codex model smoke test. Global Codex approval flags
go before the `exec` subcommand:

```bash
codex --ask-for-approval never exec \
  --sandbox read-only \
  "Reply with exactly: CODEX_LOGIN_OK"
```

Expected output includes `Logged in using ChatGPT` from `codex login status`,
then the model reply `CODEX_LOGIN_OK` from the smoke test.

OpenSymphony reports Codex subscription readiness through
`GET /api/v1/model-settings` and
`GET /api/v1/model-settings/credential-status`. The gateway probes only
supported Codex CLI surfaces:

- `codex --version`
- `codex app-server --help`
- `codex login status`

The model-settings response includes a `codex_local_readiness` summary with the
detected CLI version, app-server support, ChatGPT login state, and the safe
operator commands for login/status/logout. It also exposes the Codex profile as
a `codex_cli_login` credential reference under the existing
`codex-chatgpt-local-keychain` profile ID. That reference identifies the
operator-owned Codex CLI login state; it is not a copied access token, refresh
credential, or parsed private Codex credential payload.

The gateway caches the Codex readiness probe for a short in-process TTL so
repeated `model-settings` reads do not spawn new Codex subprocesses on every
request. Concurrent cache misses share one in-flight refresh result, and the
three Codex CLI probes run concurrently with per-probe timeouts so aggregate
readiness latency stays bounded when a local command hangs. A stalled
login-status command returns an explicit unknown/non-ready state instead of
hanging the gateway request. The readiness classifier uses command
success/failure plus the current Codex CLI status text. It treats `Logged in
using ChatGPT` and `Logged in with ChatGPT` as subscription-ready ChatGPT login
signals; logged-out, expired, unsupported, and permission-denied text are
rendered as explicit non-ready states.

Logout and revocation stay owned by Codex and ChatGPT. Run `codex logout` to
remove the local Codex login, and revoke account/device access from ChatGPT
settings when needed. If `codex login status` reports logged out, expired, an
unrecognized state, or permission denial, OpenSymphony surfaces that state
without attempting to read Codex credential files.

Run the loopback benchmark with the installed Codex binary:

```bash
node scripts/codex_app_server_benchmark.mjs \
  --iterations=10 \
  --port=18779 \
  --batch-timeout-ms=6000
```

Use `--codex-path <path>` to test a specific Codex binary. Use
`--skip-websocket` when the local Node runtime lacks global WebSocket support or
when you only need stdio evidence. The benchmark intentionally avoids
model-backed turns, so it should not consume subscription/API quota.

If you want to build one local OpenSymphony binary that includes both Codex
app-server and OpenHands ChatGPT/Codex subscription credential support, enable
the subscription feature. The Codex stdio harness itself is available in normal
builds:

```bash
cargo install --path . --no-default-features \
  --features duckdb-prebuilt,openhands-subscription-credentials
```

The subscription credential path is still owned by the model settings and
OpenHands adapter flow. Codex app-server reuses those credential-reference
profiles; it must not read or persist raw ChatGPT OAuth access or refresh tokens
inside OpenSymphony workspaces.

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
$ codex --dangerously-bypass-hook-trust app-server --stdio
request: {"jsonrpc":"2.0","id":1,"method":"initialize",...}
response: {"id":1,"result":{"userAgent":"opensymphony-probe/0.138.0 ...",
"codexHome":"/home/user/.codex","platformFamily":"unix","platformOs":"macos"}}
```

Codex CLI `0.138.0` omits the `jsonrpc` field in successful responses. The
benchmark rejects unsupported `jsonrpc` values when the field is present and
otherwise validates the observed `id` plus `result` response shape.

Schema generation is required during runtime compatibility checks:

```text
codex app-server generate-json-schema --out <dir>
codex app-server generate-ts --out <dir>
```

OpenSymphony does not pin or vendor a Codex binary. It asks the installed Codex
CLI to generate its current app-server JSON Schema, validates outbound
`initialize`, `thread/start`, and `turn/start` requests against that schema, and
fails with update guidance if the installed Codex is too old or incompatible
with the required automation fields.

The generated protocol includes `initialize`, `thread/start`, `turn/start`,
`thread/started`, `turn/started`, `turn/completed`,
`item/agentMessage/delta`, `item/started`, `item/completed`, and server-side
approval request shapes.

The benchmark loop uses `thread/loaded/list` as its queued request probe because
it exercises JSON-RPC request/response routing without starting model-backed
turns or consuming subscription/API quota.

## Benchmark Script

Run:

```bash
node scripts/codex_app_server_benchmark.mjs --iterations 10 --port 18779
```

The loopback WebSocket probe uses Node's global `WebSocket` and `fetch`
implementations and therefore requires Node.js 22 or newer. Use
`--skip-websocket` for stdio-only evidence on older Node runtimes.
Use `--codex-path <path>` to benchmark a specific Codex CLI binary instead of
the first `codex` on `PATH`.
Use `--request-timeout-ms <ms>` for single-request probes and
`--batch-timeout-ms <ms>` for the queued WebSocket request batch.

The script performs:

- stdio `initialize` latency,
- loopback WebSocket readiness via `/readyz`,
- WebSocket `initialize` latency,
- queued `thread/loaded/list` request throughput and p50/p95 latency,
- reconnect by closing the socket, opening a new socket, and initializing again,
- secure exposure checks for runtime localhost-only listener output and static
  capability-token/signed-bearer WebSocket auth flags advertised by anchored
  `codex app-server --help` option lines. The loopback benchmark does not
  perform a runtime authenticated-listener probe.

Use `--skip-websocket` when the installed Codex version lacks WebSocket support;
the flag is presence-based and does not take a value.
Queued WebSocket requests use `--batch-timeout-ms`, which defaults to
`min(300000, --request-timeout-ms + --iterations * 100)`, so the timeout remains
an explicit duration even for high-iteration runs.

Do not point the experimental WebSocket benchmark at real shared-environment
secrets. Codex WebSocket auth file paths and token hashes are passed as process
arguments, so they can be visible to local process-list inspection on some
systems.

## Local Benchmark Result

On this machine with `codex-cli 0.138.0`, stdio initialize and loopback
WebSocket probes are supported. A 10-request local run produced:

```json
{
  "generatedAt": "2026-06-20T06:50:07.988Z",
  "codexVersion": "codex-cli 0.138.0",
  "stdio": {
    "transport": "stdio",
    "initializeLatencyMs": 120.332,
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
    "port": 18779,
    "initializeLatencyMs": 1.252,
    "queuedRequests": 10,
    "queuedResponses": 10,
    "queueElapsedMs": 0.856,
    "requestsPerSecond": 11678.26,
    "latencyMs": {
      "p50": 0.475,
      "p95": 0.569,
      "max": 0.569
    },
    "reconnectLatencyMs": 0.867,
    "reconnectResponse": {
      "id": 12,
      "result": {
        "userAgent": "opensymphony-codex-benchmark/0.138.0 (Mac OS 26.4.0; arm64) dumb (opensymphony-codex-benchmark-reconnect; 0.0.0)",
        "codexHome": "/home/user/.codex",
        "platformFamily": "unix",
        "platformOs": "macos"
      }
    },
    "stdoutBytes": 0,
    "stderrBytes": 222,
    "stderrPreview": "codex app-server (WebSockets)\n  listening on: ws://127.0.0.1:18779\n  readyz: http://127.0.0.1:18779/readyz\n  healthz: http://127.0.0.1:18779/healthz\n  note: binds localhost only (use SSH port-forwarding for remote access)",
    "exposure": {
      "listener": "ws://127.0.0.1:18779",
      "observedListenerSource": "observed",
      "listenerHost": "127.0.0.1",
      "localhostOnly": true,
      "localhostOnlyEvidence": [
        "configured_loopback_listener",
        "parsed_listener_address"
      ],
      "authEvidence": "advertised_in_help",
      "runtimeAuthProbe": "not_measured_by_loopback_smoke",
      "authModesAdvertisedInHelp": [
        "capability-token",
        "signed-bearer-token"
      ]
    }
  },
  "secureExposure": {
    "transport": "websocket_secure_exposure",
    "authEvidence": "advertised_in_help",
    "helpSha256": "ebddcbae81d5d6520609ad5605d069ddaf1d4c02cc97cc99d2585757aa4364ff",
    "hasCapabilityTokenMode": true,
    "hasSignedBearerMode": true,
    "hasTokenFileFlag": true,
    "hasTokenSha256Flag": true,
    "hasSharedSecretFlag": true,
    "hasIssuerFlag": true,
    "hasAudienceFlag": true,
    "hasClockSkewFlag": true
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
repeatable developer benchmark records stable throughput, queue, reconnect,
runtime localhost exposure, authenticated-listener behavior, and schema
compatibility for the supported Codex version range.

## Model And Credential Reuse

Codex must reuse the gateway model settings shape instead of owning
subscription credentials. The current mapping is:

- `codex-chatgpt-local-keychain`: stable local Codex CLI ChatGPT login
  reference for desktop/local Codex app-server use.
- `hosted-openai-subscription-broker`: hosted broker reference for future
  hosted Codex app-server or OpenHands subscription use.
- selected model strings from `routing.model` or `OPENSYMPHONY_MODEL` are
  passed to Codex `thread/start` and `turn/start` where the installed app-server
  supports per-session/per-turn model overrides.
- when no model is selected for the Codex harness, OpenSymphony omits the model
  field and lets the Codex CLI/app-server use its own configured default, such
  as `~/.codex/config.toml`.

Gaps:

- No production Codex credential reader is implemented in this issue.
- No raw subscription token is stored in an OpenSymphony workspace or sent to
  browser clients.
- Hosted credential broker support remains a follow-up implementation.

## Readiness And Gaps

Codex app-server stdio is the supported local harness path. It still requires
an installed compatible Codex CLI and an active ChatGPT login. The gateway
surfaces unsupported CLI output, missing app-server support, logged-out,
expired, permission-denied, and unknown states as actionable non-ready statuses.
Capability discovery reports the local adapter contract and stdio runtime
surface. `opensymphony run` now attaches an alpha route decision to each worker
launch. Workflow `routing.harness: codex_app_server` or
`OPENSYMPHONY_HARNESS=codex_app_server` can select the local Codex app-server
stdio worker when the harness is available. `routing.model` or
`OPENSYMPHONY_MODEL` can pass an explicit model; otherwise Codex uses its own
configured default. Set `opensymphony run --dry-run` to emit a route preview
without launching a model-backed Codex session.

The local stdio worker launches the Codex binary from `OPENSYMPHONY_CODEX_BIN`,
or `codex` when unset, inside the issue workspace path. This remains a
trusted-environment alpha control: do not expose that environment variable to
untrusted users or hosted tenants. Before launching the worker, OpenSymphony
generates the app-server schema from that installed binary and validates the
outbound lifecycle requests. The worker drains stderr to structured logs to
avoid stdio pipe backpressure, but raw stderr is not copied into persisted
worker errors or run manifests. JSON-RPC initialize/start waits currently use
fixed alpha bounds of 30 seconds per response and 300 seconds for terminal
notification wait.

Codex approval notifications are normalized into the shared approval-center
contract, and the Codex adapter exposes the `approval/respond` request shape
plus matching audit records that the future action path will use. The live
operator-to-Codex response command path is not yet wired into the
`opensymphony run` local stdio worker, so local Codex capability discovery does
not advertise approve/reject actions yet. Approval-response forwarding through
the gateway/operator action loop remains follow-up work before approval-bearing
Codex runs are considered production-ready.

Remaining follow-up work:

- a checked-in generated schema artifact policy for future Codex protocol bumps,
- gateway/operator action wiring that forwards approval decisions to the live
  Codex stdio session,
- replay/history semantics beyond the local stdio request lifecycle (capability
  metadata currently marks history fetch, reconnect replay, and stdio
  reconciliation unavailable),
- security review of non-loopback WebSocket exposure with capability-token and
  signed-bearer modes,
- hosted Codex worker pools and hosted credential broker integration.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-476 contributed: PR #136: Enable local Codex app-server harness (merge `303ab81`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-476: Codex Production Harness Enablement

## Source refs

- COE-476

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
