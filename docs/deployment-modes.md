# Deployment Modes

This document defines the supported deployment shapes for OpenSymphony and clarifies what changes across local and hosted environments.

## 1. Design goal

OpenSymphony should have one execution contract and multiple deployment modes.

Stable contract:

- Symphony orchestration stays in Rust
- OpenHands agent-server is the execution substrate
- operations use REST
- runtime events use WebSocket
- each issue owns a deterministic workspace path
- each issue owns a stable OpenHands conversation by default
- FrankenTUI talks only to the OpenSymphony control plane

The runtime adapter should not need a redesign when moving from local development to hosted execution.

## 2. Mode A: local supervised mode

This is the MVP target.

### Topology

```text
developer machine
  ├─ opensymphony daemon
  │   ├─ orchestrator
  │   ├─ workspace manager
  │   ├─ linear adapter
  │   ├─ openhands REST + WS client
  │   └─ control-plane API
  └─ supervised subprocess
      └─ python -m openhands.agent_server --host 127.0.0.1 --port <port>
```

### Properties

- one OpenHands server subprocess per daemon process
- one issue workspace directory per active issue
- one `workspace.working_dir` per conversation
- no Docker container per workspace
- host filesystem and host process access are expected
- loopback-only bind is required by default
- launch metadata comes from the repo-local `tools/openhands-server/` wrapper
  and pin files, not a globally installed `openhands` binary

### Best use case

- easiest self-contained setup for an individual developer
- local experimentation and debugging
- CI smoke tests on trusted repositories

### Security posture

This is a trusted-machine mode. Treat it as process-level isolation, not sandbox isolation.
`opensymphony doctor` repeats that warning during setup and warns when a local
deployment points at a non-loopback OpenHands target.

## 3. Mode B: external local server mode

This mode uses the same Rust runtime adapter but skips local subprocess supervision.

### Topology

```text
developer machine
  ├─ opensymphony daemon
  └─ external OpenHands agent-server
```

Examples:

- a manually started local server
- a server launched by another supervisor
- a LAN-accessible test server on a trusted network

### Why support it

- simplifies debugging against a hand-managed server
- enables protocol and load tests without daemon-owned lifecycle
- keeps local and remote code paths aligned

### Required behavior

- do not attempt to stop the external server on daemon exit
- still create issue-scoped `working_dir` values
- still apply the same REST plus WebSocket client contract
- allow absolute `http://` or `https://` base URLs with optional path prefixes
- keep authenticated loopback targets in external mode instead of auto-starting the local supervisor
- health probing is allowed, but termination remains a no-op unless the daemon
  owns the launched child process

## 4. Mode C: hosted remote agent-server mode

This is the primary follow-on after the local MVP.

### Topology

```text
developer laptop
  ├─ opensymphony daemon or thin client
  └─ remote OpenSymphony control plane

server side
  ├─ opensymphony daemon
  ├─ remote OpenHands agent-server fleet
  ├─ remote workspace isolation layer
  └─ shared observability and auth
```

### Recommended isolation posture

For hosted execution, prefer remote or container-backed workspaces over host-local process execution.

### Why this is different from local mode

Hosted mode has different requirements:

- stronger auth and transport security
- stronger tenant isolation
- predictable workspace lifecycle at scale
- centralized logs and metrics
- organization-managed upgrades and runtime limits

### What remains the same

- issue scheduling rules
- workflow rendering rules
- workspace key derivation
- retry and reconciliation semantics
- REST plus WebSocket transport model
- optional FrankenTUI operator client

## 5. Mode comparison

| Dimension | Local supervised | External local | Hosted remote |
|---|---|---|---|
| Server lifecycle | daemon-owned subprocess | external | platform-owned |
| Bind scope | loopback | local or trusted network | network-exposed |
| Per-issue Docker | not required | not required | likely yes or remote sandbox equivalent |
| Workspace isolation | process-level | depends on server | strong isolation required |
| Auth requirement | optional by default | recommended | mandatory |
| Best for | solo development | debugging and tests | organizational rollout |

## 6. Workspace strategy by mode

## 6.1 Local MVP

Use host directories rooted under the configured workspace root:

```text
<workspace_root>/<sanitized_issue_identifier>/
```

The conversation request sets:

- `workspace.kind` to the pinned local-compatible value
- `workspace.working_dir` to the issue workspace path
- `persistence_dir` to a stable directory inside `.opensymphony/openhands/`

## 6.2 Hosted follow-on

The issue workspace path should remain stable from Symphony's point of view, but the actual backing implementation may be:

- remote container filesystem
- remote VM workspace
- remote managed sandbox API

The orchestrator must treat the workspace path as a logical path contract and avoid baking in local-path assumptions outside the workspace layer.

## 7. Transport and auth by mode

## 7.1 Local supervised

Defaults:

- `http://127.0.0.1:<port>`
- no auth by default
- no TLS by default

## 7.2 External local

Defaults:

- explicit base URL required
- path prefixes supported
- auth configurable
- TLS optional but supported if present
- doctor and the runtime treat authenticated or path-prefixed loopback targets as external rather than supervisor-owned

## 7.3 Hosted remote

Requirements:

- TLS required
- `openhands.transport.session_api_key_env` required
- current default auth shape is HTTP header plus WebSocket query-param fallback, with explicit WebSocket header mode available when the pinned server supports it
- version pinning required
- structured audit logging required

The MVP code should already expose the auth configuration hooks needed later.

## 8. What the code should abstract now

The runtime adapter should separate:

- transport config
- server lifecycle ownership
- workspace request shaping
- conversation persistence
- event streaming and recovery
- auth strategy

That makes local and hosted modes configuration changes, not architectural rewrites.

## 9. Recommended roadmap

### Phase 1

Deliver local supervised mode only.

### Phase 2

Support external local server mode for debugging and CI.

### Phase 3

Harden hosted remote mode beyond the current transport/auth support, including stronger workspace isolation and broader hosted-operations concerns.

This sequencing gives the project the fastest path to a working developer-focused MVP while preserving the right long-term boundaries.
