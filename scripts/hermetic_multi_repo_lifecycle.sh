#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")"/.. && pwd)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
OUTPUT_ROOT="${OPENSYMPHONY_HERMETIC_OUTPUT_ROOT:-${ROOT_DIR}/target/multi-repo-lifecycle}"
RUN_DIR="${OUTPUT_ROOT%/}/${RUN_ID}"
LOG_FILE="${RUN_DIR}/gate.log"
CONFIG_PATH="${OPENSYMPHONY_RELEASE_CONFIG:-}"

mkdir -p "${RUN_DIR}"
cd "${ROOT_DIR}"
export GIT_CONFIG_GLOBAL=/dev/null
export RUST_TEST_THREADS=1

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Release evidence requires a clean Git worktree." >&2
  exit 1
fi

if [[ -z "${CONFIG_PATH}" || ! -f "${CONFIG_PATH}" ]]; then
  echo "Set OPENSYMPHONY_RELEASE_CONFIG to the selected central config file." >&2
  exit 1
fi

if [[ "$(duckdb --version 2>/dev/null || true)" != v1.5.3* ]]; then
  echo "DuckDB 1.5.3 is required for the system-linked release gate." >&2
  exit 1
fi

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

run() {
  printf '\n[%s] %s\n' "$(date -u +%FT%TZ)" "$*" | tee -a "${LOG_FILE}"
  "$@" 2>&1 | tee -a "${LOG_FILE}"
}

export OPENSYMPHONY_RELEASE_CONFIG="${CONFIG_PATH}"
run cargo test-system-duckdb --lib release_candidate_selected_central_config_validates

# H01-H03: strict configuration, legacy migration/rollback, and exclusive
# process ownership. These are inherited matrices, run intact by prefix.
run cargo test-system-duckdb --lib central_config_
run cargo test-system-duckdb --lib migration_
run cargo test-system-duckdb --lib apply_and_rollback_restore_legacy_files
run cargo test-system-duckdb --lib runtime_root_ownership_
run cargo test-system-duckdb --lib strict_run_marker_
run cargo test-system-duckdb --lib orchestrator_run::backends::tests

# H04-H06: typed routing, three local bare repositories, contradictory
# instructions, retained generations, parent worktrees, and restart admission.
run cargo test-system-duckdb --test workspace_manager
run cargo test-system-duckdb --test issue_session_runner
run cargo test-system-duckdb --test client_resilience
run cargo test-system-duckdb --test codex_app_server

# H07-H10: one affected-repository repair, provider reconciliation, current-head
# review policy, final refresh, capture, Git detachment, leases, and cleanup.
# The scheduler suite includes the three-repository repair-to-capture scenario.
run cargo test-system-duckdb --test scheduler
run cargo test-system-duckdb --test linear_client
run cargo test-system-duckdb --test run

# H11: persisted memory, exact live-overlay authorization, stable grants, and
# isolated catalog coordination.
run cargo test-system-duckdb --lib central_memory_writers_share_a_catalog_coordination_lock
run cargo test-system-duckdb --lib memory_server_writer_gate_keeps_filesystem_lock_until_guards_drain
run cargo test-system-duckdb --lib memory_server_health_reports_pinned_config_generation
run cargo test-system-duckdb --lib strict_memory_context_requires_a_worker_scope_grant
run cargo test-system-duckdb --lib worker_memory_grant_
run cargo test-system-duckdb --lib parent_overlay_requires_the_verified_target_to_remain_in_head_ancestry
run cargo test-system-duckdb --lib context_records_keep_the_base_commit_for_persisted_overlay_paths
run cargo test-system-duckdb --test memory

# H12: operator projections agree across Rust and TypeScript clients.
run cargo test-system-duckdb --test snapshot_serialization operator_projection_round_trips_without_secret_or_path_fields
run cargo test-system-duckdb --test gateway_schema run_detail_roundtrips
run cargo test-system-duckdb --test gateway gateway_serves_run_detail
run cargo test-system-duckdb --test reducer
run cargo test-system-duckdb --test tui
run npm ci
run npm run type-check
run npx jest packages/gateway-schema/__tests__/fixtures.test.ts packages/api-client/__tests__/transport-contract.test.ts --runInBand
run npm run build --workspace=@opensymphony/web
run npm run build --workspace=@opensymphony/desktop

COMMIT_SHA="$(git rev-parse HEAD)"
CONFIG_SHA="$(sha256_file "${CONFIG_PATH}")"
CONFIG_PATH="$(cd -- "$(dirname -- "${CONFIG_PATH}")" && pwd)/$(basename -- "${CONFIG_PATH}")"
jq -n \
  --arg run_id "${RUN_ID}" \
  --arg commit_sha "${COMMIT_SHA}" \
  --arg config_path "${CONFIG_PATH}" \
  --arg config_sha256 "${CONFIG_SHA}" \
  --arg completed_at "$(date -u +%FT%TZ)" \
  --arg log "${LOG_FILE}" \
  '{schema_version:1,run_id:$run_id,commit_sha:$commit_sha,config_path:$config_path,config_sha256:$config_sha256,project_set:"hermetic-local",production_activation:false,result:"passed",completed_at:$completed_at,log:$log}' \
  > "${RUN_DIR}/release-evidence.json"

echo "Hermetic multi-repository lifecycle gate passed."
echo "Evidence: ${RUN_DIR}/release-evidence.json"
