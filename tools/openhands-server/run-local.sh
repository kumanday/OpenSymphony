#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v uv >/dev/null 2>&1; then
  echo "uv is required to launch the pinned OpenHands agent-server" >&2
  exit 1
fi

cd "$SCRIPT_DIR"
uv run python -m openhands.agent_server --host 127.0.0.1 --port 8000 "$@"
