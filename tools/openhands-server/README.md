# Pinned OpenHands Agent-Server

This directory pins the local trusted-machine OpenHands runtime used by the OpenSymphony MVP.

The current pin is recorded in [version.txt](./version.txt), mirrored in [pyproject.toml](./pyproject.toml), and resolved in [uv.lock](./uv.lock).

Requirements:

- `uv`
- Python `3.12+`

## Provision with `uv`

```bash
cd tools/openhands-server
uv sync
```

## Run locally

```bash
./tools/openhands-server/run-local.sh
```

The default bind is loopback-only at `127.0.0.1:8000`.
