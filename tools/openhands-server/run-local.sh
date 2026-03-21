#!/usr/bin/env bash
set -euo pipefail

HOST="${OPENHANDS_HOST:-127.0.0.1}"
PORT="${OPENHANDS_PORT:-8000}"

exec uv run python -m openhands.agent_server --host "${HOST}" --port "${PORT}"
