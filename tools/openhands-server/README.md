# OpenHands Server Pin

This directory owns the pinned local OpenHands agent-server packaging for
OpenSymphony.

Current pin:

- `version.txt` pins the OpenHands SDK bundle to `1.14.0`
- `pyproject.toml` records the direct dependency pin in
  `project.optional-dependencies.agent-server`:
  - `openhands-agent-server==1.14.0`
  - `openhands-sdk==1.14.0`
  - `openhands-tools==1.14.0`
  - `openhands-workspace==1.14.0`
- `uv.lock` records the fully resolved Python dependency graph for the local
  server launcher
- `run-local.sh` launches the pinned server via `RUNTIME=process uv run
  --directory . --locked --extra agent-server --module
  openhands.agent_server --host 127.0.0.1 --port 8000`

The wrapper owns the process-sandbox selection (`RUNTIME=process`) and loopback
bind host, and uses `OPENHANDS_SERVER_PORT` to set an explicit port when the
default `8000` needs to change. It rejects all extra agent-server CLI arguments
so local smoke runs preserve the same single-server supervised topology and
host-process execution mode as the daemon-managed path.

The local MVP uses this exact pin for:

- the local supervised server command
- HTTP and WebSocket contract verification
- doctor checks
- live local integration tests

The repo currently constrains this environment to Python `3.12.x` via
`requires-python = \">=3.12,<3.13\"` because the pinned OpenHands package line
requires Python 3.12 or newer.

Do not rely on a globally installed moving-target `openhands` binary for this
repository.
