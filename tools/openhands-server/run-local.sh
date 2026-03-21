#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

HOST="127.0.0.1"
PORT="8000"
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host)
      HOST="$2"
      shift 2
      ;;
    --port)
      PORT="$2"
      shift 2
      ;;
    *)
      EXTRA_ARGS+=("$1")
      shift
      ;;
  esac
done

exec uv run --project "$ROOT_DIR" agent-server --host "$HOST" --port "$PORT" "${EXTRA_ARGS[@]}"
