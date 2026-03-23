#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v uv >/dev/null 2>&1; then
  echo "uv is required to install the pinned OpenHands agent-server environment" >&2
  exit 1
fi

if (( $# > 0 )); then
  echo "install.sh does not accept extra arguments." >&2
  echo "It always runs: uv sync --directory <tool-dir> --locked --extra agent-server" >&2
  exit 1
fi

exec uv sync \
  --directory "${SCRIPT_DIR}" \
  --locked \
  --extra agent-server
