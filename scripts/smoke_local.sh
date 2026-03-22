#!/usr/bin/env bash
set -euo pipefail

cargo run -p opensymphony-cli -- doctor --config examples/configs/local-dev.yaml

