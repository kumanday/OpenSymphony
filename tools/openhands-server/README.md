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
uv sync --project tools/openhands-server
```

Run the server on loopback:

```bash
tools/openhands-server/run-local.sh --port 8000
```

Probe readiness:

```bash
curl http://127.0.0.1:8000/ready
```
