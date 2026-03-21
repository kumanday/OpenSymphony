# OpenHands Server Pin Placeholder

This directory reserves ownership of the local OpenHands agent-server packaging
for OpenSymphony.

Bootstrap state in M1:

- `version.txt` carries the unresolved version placeholder
- `pyproject.toml` records the future package pin in `project.optional-dependencies.agent-server`
- `uv.lock` is a placeholder that must be replaced by a resolved lockfile
- `run-local.sh` fails closed until the version, uv extra, and lockfile are all
  resolved, then launches the pinned server via `uv run --directory . --locked
  --extra agent-server --module openhands.agent_server --host 127.0.0.1 --port
  8000`

The wrapper owns the loopback bind host and uses `OPENHANDS_SERVER_PORT` to set
an explicit port when the default `8000` needs to change. It rejects all extra
agent-server CLI arguments so local smoke runs preserve the same single-server
supervised topology as the daemon-managed path.

The local MVP must eventually pin the exact OpenHands package version used for:

- the local supervised server command
- HTTP and WebSocket contract verification
- doctor checks
- live local integration tests

Do not rely on a globally installed moving-target `openhands` binary for this
repository.
