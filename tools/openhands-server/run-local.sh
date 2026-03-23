#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
HOST="127.0.0.1"
PORT="${OPENHANDS_SERVER_PORT:-8000}"

if ! command -v uv >/dev/null 2>&1; then
  echo "uv is required to launch the pinned OpenHands agent-server" >&2
  exit 1
fi

if ! [[ "${PORT}" =~ ^[0-9]+$ ]] || (( 10#${PORT} < 1 || 10#${PORT} > 65535 )); then
  echo "OPENHANDS_SERVER_PORT must be an integer between 1 and 65535." >&2
  exit 1
fi

if (( $# > 0 )); then
  echo "run-local.sh does not accept extra agent-server CLI flags in supervised mode." >&2
  echo "Use OPENHANDS_SERVER_PORT to change the local port if needed." >&2
  exit 1
fi

RUNTIME=process \
  uv run \
  --directory "${SCRIPT_DIR}" \
  --locked \
  --extra agent-server \
  --module openhands.agent_server \
  --host "${HOST}" \
  --port "${PORT}"
