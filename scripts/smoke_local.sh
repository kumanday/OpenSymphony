#!/usr/bin/env bash
set -euo pipefail

cargo run -- doctor --config examples/configs/local-dev.yaml
