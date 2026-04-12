#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if rg -n -S \
  'opensymphony-linear-mcp|linear-mcp|mcp_config|OpenHandsMcp|FIREWORKS_API_KEY|Linear MCP|\bMCP\b' \
  . \
  -g '!docs/migration-1.0.0.md' \
  -g '!crates/opensymphony-workflow/src/model.rs' \
  -g '!crates/opensymphony-workflow/src/resolve.rs' \
  -g '!crates/opensymphony-workflow/src/lib.rs' \
  -g '!scripts/check_graphql_only_cleanup.sh' \
  -g '!target' \
  -g '!target/**'
then
  echo "Found legacy bridge or provider-specific review references outside the allowed migration files." >&2
  exit 1
fi

echo "GraphQL-only cleanup check passed."
