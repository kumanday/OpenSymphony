# Pinned OpenHands Agent-Server

This directory owns the pinned local trusted-machine OpenHands runtime used by the OpenSymphony MVP.

Current pin:

- `version.txt` records the expected OpenHands SDK bundle version: `1.14.0`
- `pyproject.toml` records the pinned `agent-server` extra:
  - `openhands-agent-server==1.14.0`
  - `openhands-sdk==1.14.0`
  - `openhands-tools==1.14.0`
  - `openhands-workspace==1.14.0`
- `uv.lock` records the resolved Python dependency graph for that exact pin
- `run-local.sh` launches the pinned server via `RUNTIME=process uv run --directory . --locked --extra agent-server --module openhands.agent_server --host 127.0.0.1 --port 8000`

Requirements:

- `uv`
- Python `3.12.x`

## Provision with `uv`

```bash
cd tools/openhands-server
uv sync --extra agent-server
```

## Run locally

```bash
./tools/openhands-server/run-local.sh
```

The launcher binds to loopback-only at `127.0.0.1:8000` by default and only accepts `OPENHANDS_SERVER_PORT` as a runtime override. It intentionally rejects extra agent-server CLI flags so smoke runs stay aligned with the daemon-managed supervised topology.

Do not rely on a globally installed moving-target `openhands` binary for this repository.
