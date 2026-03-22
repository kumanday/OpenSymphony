#!/usr/bin/env bash
set -euo pipefail

export OPENSYMPHONY_LIVE_OPENHANDS="${OPENSYMPHONY_LIVE_OPENHANDS:-0}"

if [[ "${OPENSYMPHONY_LIVE_OPENHANDS}" != "1" ]]; then
  echo "Set OPENSYMPHONY_LIVE_OPENHANDS=1 to run the live local suite."
  exit 1
fi

cargo run -p opensymphony-cli -- doctor --config examples/configs/local-dev.with-live-openhands.yaml --live-openhands

