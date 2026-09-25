#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")"/.. && pwd)"
LINEAR_HELPER="${ROOT_DIR}/.agents/skills/linear/scripts/linear_graphql.py"
LINEAR_QUERIES="${ROOT_DIR}/.agents/skills/linear/queries"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
SLUG="osym-live-$(printf '%s' "${RUN_ID}" | tr '[:upper:]' '[:lower:]' | tr -cd 'a-z0-9-')"
OUTPUT_ROOT="${OPENSYMPHONY_LIVE_OUTPUT_ROOT:-${ROOT_DIR}/target/live-multi-repo}"
RUN_DIR="${OUTPUT_ROOT%/}/${RUN_ID}"
RESOURCE_DIR="${RUN_DIR}/resources"
LOG_DIR="${RUN_DIR}/logs"
CONFIG_PATH="${RUN_DIR}/config.yaml"
MANIFEST_PATH="${RUN_DIR}/resources.json"
TEARDOWN_PATH="${RUN_DIR}/teardown.json"
MAX_SECONDS="${OPENSYMPHONY_LIVE_MAX_SECONDS:-3600}"
POLL_SECONDS="${OPENSYMPHONY_LIVE_POLL_SECONDS:-10}"

REPOSITORIES=()
REPOSITORY_IDS=()
ISSUE_IDS=()
ISSUE_IDENTIFIERS=()
LABEL_IDS=()
PROJECT_ID=""
PROJECT_SLUG=""
ORCHESTRATOR_PID=""
WATCHDOG_PID=""
SCENARIO_PASSED=0

required_env=(
  GH_TOKEN
  LINEAR_API_KEY
  OPENSYMPHONY_LIVE_GITHUB_OWNER
  OPENSYMPHONY_LIVE_LINEAR_TEAM_ID
  OPENSYMPHONY_LIVE_MODEL
)

if [[ "${OPENSYMPHONY_LIVE_MULTI_REPO:-0}" != "1" ]]; then
  echo "Set OPENSYMPHONY_LIVE_MULTI_REPO=1 to run the disposable live rollout." >&2
  exit 1
fi

for variable in "${required_env[@]}"; do
  if [[ -z "${!variable:-}" ]]; then
    echo "Missing required environment variable: ${variable}" >&2
    exit 1
  fi
done

if [[ ! "${MAX_SECONDS}" =~ ^[1-9][0-9]*$ || ! "${POLL_SECONDS}" =~ ^[1-9][0-9]*$ ]]; then
  echo "Live timeout and poll interval must be positive integer seconds." >&2
  exit 1
fi

for command in cargo codex git gh jq lsof python3 shasum; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "Missing required command: ${command}" >&2
    exit 1
  fi
done

if [[ -n "$(git -C "${ROOT_DIR}" status --porcelain)" ]]; then
  echo "The live rollout requires a clean immutable candidate checkout." >&2
  exit 1
fi

if ! codex login status 2>&1 | grep -Eq 'Logged in (using|with) ChatGPT'; then
  echo "The Codex app-server harness requires an active ChatGPT login." >&2
  exit 1
fi

mkdir -p "${RESOURCE_DIR}" "${LOG_DIR}"
cd "${ROOT_DIR}"
write_intent() {
  jq -n --arg run_id "${RUN_ID}" --arg slug "${SLUG}" \
    --arg owner "${OPENSYMPHONY_LIVE_GITHUB_OWNER}" \
    --arg team "${OPENSYMPHONY_LIVE_LINEAR_TEAM_ID}" \
    '{schema_version:1,run_id:$run_id,slug:$slug,github_owner:$owner,linear_team_id:$team,
      repositories:(["alpha","beta","gamma"] | map($owner + "/" + $slug + "-" + .)),
      project_name:$slug,
      label_names:(["alpha","beta","gamma"] | map("repo:" + $slug + "-" + .)),
      issue_titles:(["parent","alpha","beta","gamma"] | map($slug + "-" + .))}' \
    > "${RUN_DIR}/intent.json"
}
write_intent
export GIT_CONFIG_GLOBAL=/dev/null
cat > "${RESOURCE_DIR}/git-askpass.sh" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  *Username*) printf '%s' 'x-access-token' ;;
  *Password*) printf '%s' "${GH_TOKEN}" ;;
  *) exit 1 ;;
esac
EOF
chmod 700 "${RESOURCE_DIR}/git-askpass.sh"
export GIT_ASKPASS="${RESOURCE_DIR}/git-askpass.sh"
export GIT_TERMINAL_PROMPT=0

linear() {
  local query_file="$1"
  local variables_file="$2"
  python3 "${LINEAR_HELPER}" \
    --query-file "${LINEAR_QUERIES}/${query_file}" \
    --variables-file "${variables_file}"
}

write_json() {
  local path="$1"
  shift
  jq -n "$@" > "${path}"
}

decode_base64() {
  python3 -c 'import base64, sys; sys.stdout.write(base64.b64decode(sys.stdin.read()).decode())'
}

record_manifest() {
  jq -n \
    --arg run_id "${RUN_ID}" \
    --arg project_id "${PROJECT_ID}" \
    --arg project_slug "${PROJECT_SLUG}" \
    --argjson repositories "$(printf '%s\n' "${REPOSITORIES[@]:-}" | jq -Rsc 'split("\n") | map(select(length > 0))')" \
    --argjson repository_ids "$(printf '%s\n' "${REPOSITORY_IDS[@]:-}" | jq -Rsc 'split("\n") | map(select(length > 0))')" \
    --argjson issue_ids "$(printf '%s\n' "${ISSUE_IDS[@]:-}" | jq -Rsc 'split("\n") | map(select(length > 0))')" \
    --argjson label_ids "$(printf '%s\n' "${LABEL_IDS[@]:-}" | jq -Rsc 'split("\n") | map(select(length > 0))')" \
    '{schema_version:1,run_id:$run_id,project:{id:$project_id,slug:$project_slug},repositories:$repositories,repository_ids:$repository_ids,issue_ids:$issue_ids,label_ids:$label_ids}' \
    > "${MANIFEST_PATH}"
}

delete_linear_resource() {
  local query="$1"
  local id="$2"
  local vars="${RUN_DIR}/delete-${query}-${id}.json"
  write_json "${vars}" --arg id "${id}" '{id:$id}'
  local result
  result="$(linear "${query}.graphql" "${vars}")" || return 1
  jq -e '.data | [.[] | .success] == [true]' <<<"${result}" >/dev/null
}

repository_is_absent() {
  local response
  response="$(gh api -i "repos/$1" 2>&1)"
  grep -Eq '^HTTP/[0-9.]+ 404 ' <<<"${response}"
}

reconcile_linear_resources() {
  local result id alias vars
  vars="${RUN_DIR}/reconcile-project.json"
  write_json "${vars}" --arg name "${SLUG}" '{name:$name}'
  result="$(linear project_by_name.graphql "${vars}")" || return 1
  id="$(jq -r '.data.projects.nodes[0].id // empty' <<<"${result}")"
  if [[ -n "${id}" ]]; then PROJECT_ID="${id}"; fi
  for alias in alpha beta gamma; do
    vars="${RUN_DIR}/reconcile-label-${alias}.json"
    write_json "${vars}" --arg name "repo:${SLUG}-${alias}" --arg team "${OPENSYMPHONY_LIVE_LINEAR_TEAM_ID}" '{name:$name,teamId:$team,first:2}'
    result="$(linear issue_label_by_name.graphql "${vars}")" || return 1
    id="$(jq -r '.data.issueLabels.nodes[0].id // empty' <<<"${result}")"
    if [[ -n "${id}" && ! " ${LABEL_IDS[*]:-} " =~ " ${id} " ]]; then LABEL_IDS+=("${id}"); fi
  done
  for alias in parent alpha beta gamma; do
    vars="${RUN_DIR}/reconcile-issue-${alias}.json"
    write_json "${vars}" --arg title "${SLUG}-${alias}" --arg team "${OPENSYMPHONY_LIVE_LINEAR_TEAM_ID}" '{title:$title,teamId:$team}'
    result="$(linear issue_by_title.graphql "${vars}")" || return 1
    id="$(jq -r '.data.issues.nodes[0].id // empty' <<<"${result}")"
    if [[ -n "${id}" && ! " ${ISSUE_IDS[*]:-} " =~ " ${id} " ]]; then ISSUE_IDS+=("${id}"); fi
  done
  record_manifest
}

cleanup() {
  local cleanup_failed=0
  set +e
  if [[ -n "${WATCHDOG_PID}" ]] && kill -0 "${WATCHDOG_PID}" 2>/dev/null; then
    kill "${WATCHDOG_PID}" 2>/dev/null
    wait "${WATCHDOG_PID}" 2>/dev/null
  fi
  if [[ -n "${ORCHESTRATOR_PID}" ]] && kill -0 "${ORCHESTRATOR_PID}" 2>/dev/null; then
    kill -TERM "${ORCHESTRATOR_PID}" 2>/dev/null
    for _ in $(seq 1 20); do
      kill -0 "${ORCHESTRATOR_PID}" 2>/dev/null || break
      sleep 1
    done
    kill -KILL "${ORCHESTRATOR_PID}" 2>/dev/null
    wait "${ORCHESTRATOR_PID}" 2>/dev/null
  fi

  # Reconcile deterministic names before deleting: a create can succeed remotely
  # even when its response never reaches this process.
  reconcile_linear_resources || cleanup_failed=1
  for ((index=${#ISSUE_IDS[@]}-1; index>=0; index--)); do
    delete_linear_resource issue_delete "${ISSUE_IDS[index]}" || cleanup_failed=1
  done
  for ((index=${#LABEL_IDS[@]}-1; index>=0; index--)); do
    delete_linear_resource issue_label_delete "${LABEL_IDS[index]}" || cleanup_failed=1
  done
  if [[ -n "${PROJECT_ID}" ]]; then
    delete_linear_resource project_delete "${PROJECT_ID}" || cleanup_failed=1
  fi
  for ((index=${#REPOSITORIES[@]}-1; index>=0; index--)); do
    gh repo delete "${REPOSITORIES[index]}" --yes >/dev/null 2>&1 || {
      repository_is_absent "${REPOSITORIES[index]}" || cleanup_failed=1
    }
  done
  for repository in "${REPOSITORIES[@]:-}"; do
    [[ -z "${repository}" ]] && continue
    repository_is_absent "${repository}" || cleanup_failed=1
  done

  rm -rf "${RESOURCE_DIR}/seeds" "${RUN_DIR}/state" "${RUN_DIR}/workspaces" "${RUN_DIR}/catalog"
  local remaining_processes=0
  if [[ -n "${ORCHESTRATOR_PID}" ]] && kill -0 "${ORCHESTRATOR_PID}" 2>/dev/null; then
    remaining_processes=1
    cleanup_failed=1
  fi
  local remaining_port_owner=0
  if [[ -n "${PORT:-}" ]] && lsof -tiTCP:"${PORT}" -sTCP:LISTEN >/dev/null 2>&1; then
    remaining_port_owner=1
    cleanup_failed=1
  fi
  jq -n \
    --arg run_id "${RUN_ID}" \
    --argjson scenario_passed "${SCENARIO_PASSED}" \
    --argjson cleanup_failed "${cleanup_failed}" \
    --argjson remaining_processes "${remaining_processes}" \
    --argjson remaining_port_owner "${remaining_port_owner}" \
    --arg completed_at "$(date -u +%FT%TZ)" \
    '{schema_version:1,run_id:$run_id,scenario_passed:($scenario_passed == 1),cleanup_complete:($cleanup_failed == 0),remaining_processes:$remaining_processes,remaining_port_owner:$remaining_port_owner,remaining_local_roots:0,credential_copies:0,completed_at:$completed_at}' \
    > "${TEARDOWN_PATH}"
  set -e
  if (( cleanup_failed != 0 )); then
    echo "Live rollout teardown was incomplete: ${TEARDOWN_PATH}" >&2
    return 1
  fi
}
trap 'status=$?; trap - EXIT; cleanup || status=1; exit "${status}"' EXIT

PORT="$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)"

create_repository() {
  local alias="$1"
  local repository="${OPENSYMPHONY_LIVE_GITHUB_OWNER}/${SLUG}-${alias}"
  local seed="${RESOURCE_DIR}/seeds/${alias}"
  REPOSITORIES+=("${repository}")
  record_manifest
  gh repo create "${repository}" --private --disable-issues --disable-wiki >/dev/null
  REPOSITORY_IDS+=("$(gh api "repos/${repository}" --jq '.id | tostring')")
  mkdir -p "${seed}/scripts" "${seed}/.github/workflows"
  git -C "${seed}" init -b develop >/dev/null
  cat > "${seed}/AGENTS.md" <<EOF
# Disposable OpenSymphony live fixture: ${alias}

Work only in this repository. Create delivery.txt with the exact content
delivered:${alias}:${RUN_ID} and run ./scripts/check.sh. Leave the edit in
this checkout for the rollout controller to commit, push, and open the pull
request. Do not perform Git or provider side effects.
EOF
  printf 'component=%s\n' "${alias}" > "${seed}/component.txt"
  if [[ "${alias}" == "alpha" ]]; then
    printf 'answer=41\n' > "${seed}/answer.txt"
  else
    printf 'answer=42\n' > "${seed}/answer.txt"
  fi
  cat > "${seed}/scripts/check.sh" <<EOF
#!/usr/bin/env bash
set -euo pipefail
test "\$(cat component.txt)" = "component=${alias}"
test "\$(cat delivery.txt)" = "delivered:${alias}:${RUN_ID}"
EOF
  if [[ "${alias}" == "alpha" ]]; then
    cat >> "${seed}/scripts/check.sh" <<EOF
if [[ "\${GITHUB_ACTIONS:-}" == "true" ]]; then
  test "\$(cat reviewed.txt 2>/dev/null)" = "reviewed:${RUN_ID}" || {
    echo "Create reviewed.txt containing reviewed:${RUN_ID}, then push the same PR branch." >&2
    exit 1
  }
fi
EOF
  fi
  chmod +x "${seed}/scripts/check.sh"
  cat > "${seed}/.github/workflows/check.yml" <<'EOF'
name: fixture-check
on:
  pull_request:
    branches: [develop]
permissions:
  contents: read
jobs:
  check:
    runs-on: ubuntu-latest
    timeout-minutes: 5
    steps:
      - uses: actions/checkout@v4
      - run: ./scripts/check.sh
EOF
  git -C "${seed}" add .
  git -C "${seed}" -c user.name='OpenSymphony Live Gate' -c user.email='live-gate@invalid.example' commit -m 'Initialize disposable fixture' >/dev/null
  git -C "${seed}" remote add origin "https://github.com/${repository}.git"
  git -C "${seed}" push -u origin develop >/dev/null
  gh api -X PATCH "repos/${repository}" -f default_branch=develop >/dev/null
}

for alias in alpha beta gamma; do
  create_repository "${alias}"
done
record_manifest

project_vars="${RUN_DIR}/project-create.json"
write_json "${project_vars}" \
  --arg name "${SLUG}" \
  --arg team "${OPENSYMPHONY_LIVE_LINEAR_TEAM_ID}" \
  '{input:{name:$name,teamIds:[$team],description:"Disposable OpenSymphony multi-repository rollout fixture"}}'
project_result="$(linear project_create.graphql "${project_vars}")"
PROJECT_ID="$(jq -er '.data.projectCreate.project.id' <<<"${project_result}")"
PROJECT_SLUG="$(jq -er '.data.projectCreate.project.slugId' <<<"${project_result}")"
record_manifest

create_label() {
  local alias="$1"
  local vars="${RUN_DIR}/label-${alias}.json"
  write_json "${vars}" \
    --arg name "repo:${alias}" \
    --arg team "${OPENSYMPHONY_LIVE_LINEAR_TEAM_ID}" \
    '{input:{name:$name,teamId:$team,color:"#5E6AD2",description:"Disposable repository binding"}}'
  local result
  result="$(linear issue_label_create.graphql "${vars}")"
  LABEL_IDS+=("$(jq -er '.data.issueLabelCreate.issueLabel.id' <<<"${result}")")
}
for alias in alpha beta gamma; do
  create_label "${SLUG}-${alias}"
done
record_manifest

create_issue() {
  local alias="$1"
  local label_id="$2"
  local parent_id="${3:-}"
  local vars="${RUN_DIR}/issue-${alias}.json"
  local title="${SLUG}-${alias}"
  local description="Run the bounded disposable task described by this repository's AGENTS.md. Do not touch another repository."
  if [[ "${alias}" == "parent" ]]; then
    title="${SLUG}-parent"
    description="Integrate all three terminal children. Run the checked-in integration instructions, repair only alpha's seeded answer defect, and leave beta and gamma unchanged."
  fi
  jq -n \
    --arg team "${OPENSYMPHONY_LIVE_LINEAR_TEAM_ID}" \
    --arg title "${title}" \
    --arg description "${description}" \
    --arg project "${PROJECT_ID}" \
    --arg parent "${parent_id}" \
    --arg label "${label_id}" \
    '{input:{teamId:$team,title:$title,description:$description,projectId:$project} + (if $parent == "" then {} else {parentId:$parent} end) + (if $label == "" then {} else {labelIds:[$label]} end)}' \
    > "${vars}"
  local result
  result="$(linear issue_create.graphql "${vars}")"
  jq -cer '.data.issueCreate.issue | {id,identifier}' <<<"${result}"
}

parent_result="$(create_issue parent "")"
PARENT_ID="$(jq -er .id <<<"${parent_result}")"
PARENT_IDENTIFIER="$(jq -er .identifier <<<"${parent_result}")"
ISSUE_IDS+=("${PARENT_ID}")
ISSUE_IDENTIFIERS+=("${PARENT_IDENTIFIER}")
CHILD_IDS=()
CHILD_IDENTIFIERS=()
for index in 0 1 2; do
  alias=(alpha beta gamma)
  child_result="$(create_issue "${alias[index]}" "${LABEL_IDS[index]}" "${PARENT_ID}")"
  child_id="$(jq -er .id <<<"${child_result}")"
  child_identifier="$(jq -er .identifier <<<"${child_result}")"
  ISSUE_IDS+=("${child_id}")
  ISSUE_IDENTIFIERS+=("${child_identifier}")
  CHILD_IDS+=("${child_id}")
  CHILD_IDENTIFIERS+=("${child_identifier}")
done
record_manifest

state_vars="${RUN_DIR}/team-states.json"
write_json "${state_vars}" --arg id "${PARENT_ID}" '{id:$id}'
state_result="$(linear issue_team_states.graphql "${state_vars}")"
state_id() {
  local name="$1"
  jq -er --arg name "${name}" '.data.issue.team.states.nodes[] | select(.name == $name) | .id' <<<"${state_result}"
}
TODO_STATE="$(state_id Todo)"
HUMAN_REVIEW_STATE="$(state_id 'Human Review')"
REWORK_STATE="$(state_id Rework)"
DONE_STATE="$(state_id Done)"

move_issue() {
  local issue_id="$1"
  local target_state="$2"
  local vars="${RUN_DIR}/move-${issue_id}.json"
  write_json "${vars}" --arg id "${issue_id}" --arg state "${target_state}" '{id:$id,stateId:$state}'
  linear issue_move_to_state.graphql "${vars}" | jq -e '.data.issueUpdate.success == true' >/dev/null
}
for issue_id in "${ISSUE_IDS[@]}"; do
  move_issue "${issue_id}" "${TODO_STATE}"
done

cat > "${RUN_DIR}/integration.md" <<EOF
# Disposable integration instructions

Operate only on the three verified repository handles. Run each repository's
./scripts/check.sh. The final integrated value in each answer.txt must be
answer=42. Alpha is intentionally seeded with answer=41: repair only alpha,
leave the verified checkout changes for OpenSymphony to publish, and do not
perform Git or provider side effects yourself. Do not modify beta or gamma.
Re-run all three checks before reporting completion.
EOF

cat > "${CONFIG_PATH}" <<EOF
schema_version: 1
instance:
  id: ${SLUG}
  state_root: ${RUN_DIR}/state
routing:
  mode: project_set
  active_project_set: live
  harness: codex_app_server
  model: ${OPENSYMPHONY_LIVE_MODEL}
tracker_profiles:
  linear:
    provider: linear
    endpoint: https://api.linear.app/graphql
    credential: linear-key
    active_states: [Todo, In Progress, Rework]
    terminal_states: [Done, Canceled]
project_sets:
  live:
    tracker_profile: linear
    integration_instructions: integration.md
    projects: [live-project]
linear_projects:
  live-project:
    provider_project_id: ${PROJECT_ID}
    provider_project_slug: ${PROJECT_SLUG}
    repositories: [alpha, beta, gamma]
repositories:
EOF
for index in 0 1 2; do
  alias=(alpha beta gamma)
  repository="${REPOSITORIES[index]}"
  repository_id="${REPOSITORY_IDS[index]}"
  cat >> "${CONFIG_PATH}" <<EOF
  ${alias[index]}:
    aliases: [${SLUG}-${alias[index]}]
    remote:
      provider: github
      provider_id: "${repository_id}"
      locator: ${repository}
      clone: https://github.com/${repository}.git
    target_branch: develop
    credential: github-clone
    review_profile: github-review
    instructions:
      path: AGENTS.md
EOF
done
cat >> "${CONFIG_PATH}" <<EOF
credentials:
  linear-key:
    kind: environment
    variable: LINEAR_API_KEY
  github-clone:
    kind: environment
    variable: GH_TOKEN
  github-review-token:
    kind: environment
    variable: GH_TOKEN
review_profiles:
  github-review:
    provider: github
    credential: github-review-token
    required_checks: true
    required_review: false
    merge_method: squash
workspace:
  root: ${RUN_DIR}/workspaces
  retain_failed: true
  cleanup_after_parent_finalization: true
memory:
  catalog_root: ${RUN_DIR}/catalog
  auto_capture: true
  auto_archive: false
  serve: false
scheduler:
  max_concurrent_tasks: 3
  max_concurrent_agents_by_state:
    Todo: 3
    In Progress: 3
    Rework: 1
  retry:
    max_attempts: 2
  poll_interval_ms: 2000
  max_turns: 8
  max_retry_backoff_ms: 10000
  stall_timeout_ms: 300000
integration:
  policy: parent_repair_prs
  use_shared_git_worktrees: true
control_plane:
  bind: 127.0.0.1:${PORT}
EOF

OPENSYMPHONY_RELEASE_CONFIG="${CONFIG_PATH}" \
OPENSYMPHONY_HERMETIC_OUTPUT_ROOT="${RUN_DIR}/hermetic" \
  "${ROOT_DIR}/scripts/hermetic_multi_repo_lifecycle.sh" >"${LOG_DIR}/hermetic.log" 2>&1

COMMIT_SHA="$(git rev-parse HEAD)"
CONFIG_SHA="$(shasum -a 256 "${CONFIG_PATH}" | awk '{print $1}')"
HERMETIC_EVIDENCE="$(find "${RUN_DIR}/hermetic" -name release-evidence.json -print -quit)"
jq -e --arg commit "${COMMIT_SHA}" --arg config "${CONFIG_SHA}" \
  '.result == "passed" and .commit_sha == $commit and .config_sha256 == $config' \
  "${HERMETIC_EVIDENCE}" >/dev/null
jq -n \
  --arg run_id "${RUN_ID}" \
  --arg commit_sha "${COMMIT_SHA}" \
  --arg config_sha256 "${CONFIG_SHA}" \
  --arg hermetic_evidence "${HERMETIC_EVIDENCE}" \
  --arg project_id "${PROJECT_ID}" \
  --arg project_slug "${PROJECT_SLUG}" \
  --arg port "${PORT}" \
  '{schema_version:1,run_id:$run_id,commit_sha:$commit_sha,config_sha256:$config_sha256,hermetic_evidence:$hermetic_evidence,project_set:"live",linear_project:{id:$project_id,slug:$project_slug},control_plane_port:($port|tonumber),production_activation:false,result:"started"}' \
  > "${RUN_DIR}/release-evidence.json"

cargo run -- run --config "${CONFIG_PATH}" >"${LOG_DIR}/orchestrator.log" 2>&1 &
ORCHESTRATOR_PID=$!
(
  sleep "${MAX_SECONDS}"
  if kill -0 "${ORCHESTRATOR_PID}" 2>/dev/null; then
    echo "Live rollout exceeded ${MAX_SECONDS} seconds." >>"${LOG_DIR}/watchdog.log"
    kill -TERM "${ORCHESTRATOR_PID}" 2>/dev/null || true
  fi
) &
WATCHDOG_PID=$!

attach_pr() {
  local issue_id="$1"
  local url="$2"
  local title="$3"
  local vars="${RUN_DIR}/attach-${issue_id}.json"
  write_json "${vars}" --arg issue "${issue_id}" --arg url "${url}" --arg title "${title}" '{issueId:$issue,url:$url,title:$title}'
  linear attachment_link_github_pr.graphql "${vars}" | jq -e '.data.attachmentLinkGitHubPR.success == true' >/dev/null
}

publish_child_if_ready() {
  local index="$1"
  local alias=(alpha beta gamma)
  local repository="${REPOSITORIES[index]}"
  local branch="feat/${SLUG}-${alias[index]}"
  local checkout candidate path
  checkout=""
  for candidate in "${RUN_DIR}/workspaces/${CHILD_IDENTIFIERS[index]}-"*--*; do
    [[ -d "${candidate}/.git" ]] || continue
    [[ -z "${checkout}" ]] || { echo "Multiple retained child checkouts for ${CHILD_IDENTIFIERS[index]}" >&2; return 1; }
    checkout="${candidate}"
  done
  [[ -n "${checkout}" ]] || return 0
  jq -e --arg id "${CHILD_IDS[index]}" '.issue_id == $id and .status == "succeeded"' \
    "${checkout}/.opensymphony/run.json" >/dev/null 2>&1 || return 0
  [[ "$(cat "${checkout}/delivery.txt" 2>/dev/null || true)" == "delivered:${alias[index]}:${RUN_ID}" ]] || return 0
  if (( index == 0 && ALPHA_REWORK_REQUIRED == 1 )); then
    [[ "$(cat "${checkout}/reviewed.txt" 2>/dev/null || true)" == "reviewed:${RUN_ID}" ]] || return 0
  elif gh pr view "${branch}" --repo "${repository}" >/dev/null 2>&1; then
    return 0
  fi
  [[ "$(git -C "${checkout}" remote get-url origin)" == "https://github.com/${repository}.git" ]] || {
    echo "Child checkout origin changed for ${CHILD_IDENTIFIERS[index]}" >&2; return 1;
  }
  while IFS= read -r path; do
    [[ "${path}" == delivery.txt || "${path}" == reviewed.txt ]] || {
      echo "Unexpected child checkout edit: ${path}" >&2; return 1;
    }
  done < <(git -C "${checkout}" status --porcelain --untracked-files=all | cut -c4-)
  git -C "${checkout}" add -- delivery.txt
  if [[ -f "${checkout}/reviewed.txt" ]]; then git -C "${checkout}" add -- reviewed.txt; fi
  if ! git -C "${checkout}" diff --cached --quiet; then
    git -C "${checkout}" -c user.name='OpenSymphony Live Gate' -c user.email='live-gate@invalid.example' \
      commit -m "Complete ${alias[index]} disposable task" >/dev/null
  fi
  git -C "${checkout}" push origin "HEAD:refs/heads/${branch}" >/dev/null
  if ! gh pr view "${branch}" --repo "${repository}" >/dev/null 2>&1; then
    gh pr create --repo "${repository}" --base develop --head "${branch}" \
      --title "${SLUG}-${alias[index]} disposable delivery" \
      --body "Disposable isolated lifecycle fixture for ${CHILD_IDENTIFIERS[index]}." >/dev/null
  fi
}

checks_are_green() {
  local repository="$1"
  local number="$2"
  gh pr view "${number}" --repo "${repository}" --json statusCheckRollup | jq -r '
    [.statusCheckRollup[] | if .__typename == "CheckRun" then .conclusion else .state end] as $checks
    | ($checks | length) > 0 and all($checks[]; . == "SUCCESS" or . == "NEUTRAL" or . == "SKIPPED")
  '
}

checks_have_failed() {
  local repository="$1"
  local number="$2"
  gh pr view "${number}" --repo "${repository}" --json statusCheckRollup | jq -r '
    any(.statusCheckRollup[]; if .__typename == "CheckRun" then
      .conclusion == "FAILURE"
    else
      .state == "FAILURE" or .state == "ERROR"
    end)
  '
}

declare -a CHILD_ATTACHED=(0 0 0)
declare -a CHILD_MERGED=(0 0 0)
ALPHA_REWORK_REQUIRED=0
ALPHA_FAILED_SHA=""
PARENT_ATTACHED=0
PARENT_DONE=0
PARENT_MERGED=0
START_SECONDS="${SECONDS}"

parent_controller_is_complete() {
  local state_path="${RUN_DIR}/workspaces/.opensymphony-orchestrator-state.json"
  [[ -f "${state_path}" ]] || return 1
  jq -e --arg parent_id "${PARENT_ID}" '
    .parent_integrations[$parent_id] as $controller
    | $controller.state == "completed"
      and $controller.final_evidence != null
      and ($controller.repair_attempts | length) == 1
      and all($controller.repair_attempts[]; .status == "completed")
  ' "${state_path}" >/dev/null
}

parent_final_verification_passed() {
  local state_path="${RUN_DIR}/workspaces/.opensymphony-orchestrator-state.json"
  [[ -f "${state_path}" ]] || return 1
  jq -e --arg parent_id "${PARENT_ID}" '
    .parent_integrations[$parent_id] as $controller
    | $controller.state == "integrating"
      and $controller.attempts[-1].status == "passed"
      and ($controller.repair_attempts | length) == 1
      and all($controller.repair_attempts[]; .status == "completed")
  ' "${state_path}" >/dev/null
}

while (( SECONDS - START_SECONDS < MAX_SECONDS )); do
  if ! kill -0 "${ORCHESTRATOR_PID}" 2>/dev/null; then
    echo "OpenSymphony exited before the live lifecycle completed." >&2
    wait "${ORCHESTRATOR_PID}" || true
    exit 1
  fi

  for index in 0 1 2; do
    (( CHILD_MERGED[index] == 0 )) || continue
    publish_child_if_ready "${index}"
    alias=(alpha beta gamma)
    repository="${REPOSITORIES[index]}"
    branch="feat/${SLUG}-${alias[index]}"
    if ! pr="$(gh pr view "${branch}" --repo "${repository}" --json number,url,title,headRefOid,state 2>/dev/null)"; then
      continue
    fi
    pr_number="$(jq -er .number <<<"${pr}")"
    pr_url="$(jq -er .url <<<"${pr}")"
    pr_title="$(jq -er .title <<<"${pr}")"
    pr_sha="$(jq -er .headRefOid <<<"${pr}")"
    if (( CHILD_ATTACHED[index] == 0 )); then
      attach_pr "${CHILD_IDS[index]}" "${pr_url}" "${pr_title}"
      move_issue "${CHILD_IDS[index]}" "${HUMAN_REVIEW_STATE}"
      CHILD_ATTACHED[index]=1
    fi

    if (( index == 0 && ALPHA_REWORK_REQUIRED == 0 )); then
      if [[ "$(checks_have_failed "${repository}" "${pr_number}")" != "true" ]]; then
        continue
      fi
      ALPHA_REWORK_REQUIRED=1
      ALPHA_FAILED_SHA="${pr_sha}"
      move_issue "${CHILD_IDS[index]}" "${REWORK_STATE}"
      feedback_vars="${RUN_DIR}/alpha-rework-comment.json"
      write_json "${feedback_vars}" --arg id "${CHILD_IDS[index]}" \
        --arg body "The fixture-check GitHub Actions job failed on this PR. Read its log, create reviewed.txt containing reviewed:${RUN_ID}, rerun ./scripts/check.sh, and leave the edit in this checkout for the rollout controller to publish to the same PR branch." \
        '{issueId:$id,body:$body}'
      linear comment_create.graphql "${feedback_vars}" | jq -e '.data.commentCreate.success == true' >/dev/null
      continue
    fi
    if (( index == 0 )) && [[ "${pr_sha}" == "${ALPHA_FAILED_SHA}" ]]; then
      continue
    fi
    if (( index == 0 )); then
      reviewed_value="$(gh api "repos/${repository}/contents/reviewed.txt?ref=${branch}" --jq .content 2>/dev/null | tr -d '\n' | decode_base64 || true)"
      answer_value="$(gh api "repos/${repository}/contents/answer.txt?ref=${branch}" --jq .content 2>/dev/null | tr -d '\n' | decode_base64 || true)"
      if [[ "${reviewed_value}" != "reviewed:${RUN_ID}" || "${answer_value}" != 'answer=41' ]]; then
        continue
      fi
    fi
    if [[ "$(checks_are_green "${repository}" "${pr_number}")" != "true" ]]; then
      continue
    fi
    gh pr merge "${pr_number}" --repo "${repository}" --squash --delete-branch
    move_issue "${CHILD_IDS[index]}" "${DONE_STATE}"
    CHILD_MERGED[index]=1
  done

  if (( CHILD_MERGED[0] == 1 && CHILD_MERGED[1] == 1 && CHILD_MERGED[2] == 1 && PARENT_MERGED == 0 )); then
    repository="${REPOSITORIES[0]}"
    pr="$(gh pr list --repo "${repository}" --state all --base develop --json number,url,title,headRefName,state,statusCheckRollup \
      --jq '[.[] | select(.headRefName | startswith("fix/"))][0]')"
    if [[ "${pr}" != "null" ]]; then
      pr_number="$(jq -er .number <<<"${pr}")"
      pr_state="$(jq -er .state <<<"${pr}")"
      if (( PARENT_ATTACHED == 0 )); then
        attach_pr "${PARENT_ID}" "$(jq -er .url <<<"${pr}")" "$(jq -er .title <<<"${pr}")"
        PARENT_ATTACHED=1
      fi
      if [[ "${pr_state}" == "MERGED" ]] && (( PARENT_DONE == 0 )) && parent_final_verification_passed; then
        move_issue "${PARENT_ID}" "${DONE_STATE}"
        PARENT_DONE=1
      fi
      if [[ "${pr_state}" == "MERGED" ]] && parent_controller_is_complete; then
        jq --arg parent_id "${PARENT_ID}" '.parent_integrations[$parent_id]' \
          "${RUN_DIR}/workspaces/.opensymphony-orchestrator-state.json" \
          > "${RUN_DIR}/parent-controller-complete.json"
        PARENT_MERGED=1
      fi
    fi
  fi

  if (( PARENT_MERGED == 1 )); then
    for _ in $(seq 1 120); do
      if [[ ! -d "${RUN_DIR}/workspaces" ]] || [[ -z "$(find "${RUN_DIR}/workspaces" -mindepth 1 -maxdepth 1 ! -name '.*' -print -quit)" ]]; then
        break
      fi
      sleep 1
    done
    break
  fi
  sleep "${POLL_SECONDS}"
done

if (( PARENT_MERGED != 1 || ALPHA_REWORK_REQUIRED != 1 )); then
  echo "The bounded live lifecycle did not complete before its deadline." >&2
  exit 1
fi

if [[ -d "${RUN_DIR}/workspaces" ]] && [[ -n "$(find "${RUN_DIR}/workspaces" -mindepth 1 -maxdepth 1 ! -name '.*' -print -quit)" ]]; then
  echo "OpenSymphony did not complete eligible workspace cleanup." >&2
  exit 1
fi

if [[ "$(gh api "repos/${REPOSITORIES[0]}/contents/answer.txt?ref=develop" --jq .content | tr -d '\n' | decode_base64)" != 'answer=42' ]]; then
  echo "The affected repository did not contain the repaired value on develop." >&2
  exit 1
fi
for index in 1 2; do
  if [[ "$(gh api "repos/${REPOSITORIES[index]}/contents/answer.txt?ref=develop" --jq .content | tr -d '\n' | decode_base64)" != 'answer=42' ]]; then
    echo "An unaffected repository changed its expected integration value." >&2
    exit 1
  fi
done

jq -n \
  --arg run_id "${RUN_ID}" \
  --arg parent "${PARENT_IDENTIFIER}" \
  --arg failed_check_sha "${ALPHA_FAILED_SHA}" \
  --argjson child_merges "$(printf '%s\n' "${CHILD_MERGED[@]}" | jq -Rsc 'split("\n") | map(select(length > 0) | tonumber)')" \
  '{schema_version:1,run_id:$run_id,result:"passed",parent:$parent,child_merges:$child_merges,failed_check_sha:$failed_check_sha,parent_repair_merge:true,cleanup_handoff:true}' \
  > "${RUN_DIR}/live-rollout-summary.json"

kill -TERM "${ORCHESTRATOR_PID}" 2>/dev/null || true
wait "${ORCHESTRATOR_PID}" 2>/dev/null || true
ORCHESTRATOR_PID=""
kill "${WATCHDOG_PID}" 2>/dev/null || true
wait "${WATCHDOG_PID}" 2>/dev/null || true
WATCHDOG_PID=""

SCENARIO_PASSED=1
jq \
  --arg completed_at "$(date -u +%FT%TZ)" \
  '.result="passed" | .completed_at=$completed_at' \
  "${RUN_DIR}/release-evidence.json" > "${RUN_DIR}/release-evidence.tmp"
mv "${RUN_DIR}/release-evidence.tmp" "${RUN_DIR}/release-evidence.json"

echo "Disposable live rollout passed; teardown will now remove all resources."
echo "Evidence: ${RUN_DIR}/release-evidence.json"
