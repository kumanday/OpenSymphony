#!/usr/bin/env bash
set -euo pipefail

exec python -m openhands.agent_server --host 127.0.0.1 --port "${OPENHANDS_PORT:-8000}"
