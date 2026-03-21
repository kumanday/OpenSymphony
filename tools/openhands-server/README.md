# OpenHands Server Pin Placeholder

This directory reserves ownership of the local OpenHands agent-server packaging
for OpenSymphony.

Bootstrap state in M1:

- `version.txt` carries the unresolved version placeholder
- `pyproject.toml` records the future package pin in `project.optional-dependencies.agent-server`
- `uv.lock` is a placeholder that must be replaced by a resolved lockfile
- `run-local.sh` fails closed until the version, uv extra, and lockfile are all
  resolved, then launches the pinned server via `uv run --locked --extra
  agent-server -m openhands.agent_server`

The local MVP must eventually pin the exact OpenHands package version used for:

- the local supervised server command
- HTTP and WebSocket contract verification
- doctor checks
- live local integration tests

Do not rely on a globally installed moving-target `openhands` binary for this
repository.
