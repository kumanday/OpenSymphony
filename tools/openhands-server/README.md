# OpenHands Server Pin

This directory pins the exact OpenHands SDK agent-server packages used by the local MVP runtime adapter.

Pinned package set:

- `openhands-agent-server==1.14.0`
- `openhands-sdk==1.14.0`
- `openhands-tools==1.14.0`
- `openhands-workspace==1.14.0`

Verified contract assumptions for this pin:

- supervised entrypoint: `agent-server` or `python -m openhands.agent_server`
- REST base: `/api`
- health endpoints: `/health` and `/ready`
- event stream: `/sockets/events/{conversation_id}`
- WebSocket readiness barrier: first `ConversationStateUpdateEvent`
- REST auth header: `X-Session-API-Key`
- WebSocket auth: `X-Session-API-Key` header or `session_api_key` query parameter

## Local usage

Install and sync the pinned environment:

```bash
uv sync --project tools/openhands-server --extra agent-server
```

Run the server on loopback:

```bash
OPENHANDS_SERVER_PORT=8000 tools/openhands-server/run-local.sh
```

Probe readiness:

```bash
curl http://127.0.0.1:8000/ready
```

The wrapper always launches the pinned server in host-process mode with `RUNTIME=process`,
binds to loopback, and rejects extra agent-server CLI arguments so local smoke runs match the
daemon-managed topology.
