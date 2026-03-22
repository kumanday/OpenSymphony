#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
HOST="${OPENHANDS_HOST:-127.0.0.1}"
PORT="${OPENHANDS_PORT:-8000}"

cd "${SCRIPT_DIR}"
exec uv run python -m openhands.agent_server --host "${HOST}" --port "${PORT}"
