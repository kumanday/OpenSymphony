#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")"/.. && pwd)"
LINEAR_HELPER="${ROOT_DIR}/.agents/skills/linear/scripts/linear_graphql.py"
LINEAR_QUERIES="${ROOT_DIR}/.agents/skills/linear/queries"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
SLUG="osym-live-$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
OUTPUT_ROOT="${OPENSYMPHONY_LIVE_OUTPUT_ROOT:-${ROOT_DIR}/target/live-multi-repo}"
RUN_DIR="${OUTPUT_ROOT%/}/${RUN_ID}"
RESOURCE_DIR="${RUN_DIR}/resources"
LOG_DIR="${RUN_DIR}/logs"
CONFIG_PATH="${RUN_DIR}/config.yaml"
MANIFEST_PATH="${RUN_DIR}/resources.json"
TEARDOWN_PATH="${RUN_DIR}/teardown.json"
MAX_SECONDS="${OPENSYMPHONY_LIVE_MAX_SECONDS:-3600}"
CLEANUP_MAX_SECONDS="${OPENSYMPHONY_LIVE_CLEANUP_MAX_SECONDS:-300}"
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
HERMETIC_SUPERVISOR_PID=""
PORT_RESERVER_PID=""
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

if [[ ! "${MAX_SECONDS}" =~ ^[1-9][0-9]*$ || ! "${CLEANUP_MAX_SECONDS}" =~ ^[1-9][0-9]*$ || ! "${POLL_SECONDS}" =~ ^[1-9][0-9]*$ ]]; then
  echo "Live, cleanup, and poll intervals must be positive integer seconds." >&2
  exit 1
fi

for command in cargo codex curl git gh jq lsof python3 shasum; do
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

START_SECONDS="${SECONDS}"
DEADLINE_ACTIVE=1
deadline_command() {
  local remaining
  if (( DEADLINE_ACTIVE )); then
    remaining=$((MAX_SECONDS - (SECONDS - START_SECONDS)))
  else
    remaining=$((CLEANUP_MAX_SECONDS - (SECONDS - CLEANUP_START_SECONDS)))
  fi
  if (( remaining <= 0 )); then
    echo "Live rollout deadline expired." >&2
    return 124
  fi
  python3 - "${remaining}" "$@" <<'PY'
import os
import signal
import subprocess
import sys

def interrupted(signum, _frame):
    raise SystemExit(128 + signum)

signal.signal(signal.SIGINT, interrupted)
signal.signal(signal.SIGTERM, interrupted)
child = subprocess.Popen(sys.argv[2:], start_new_session=True)
try:
    sys.exit(child.wait(timeout=int(sys.argv[1])))
except subprocess.TimeoutExpired:
    print("Live rollout deadline expired during external command.", file=sys.stderr)
    sys.exit(124)
finally:
    if child.poll() is None:
        try:
            os.killpg(child.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
PY
}

gh() {
  deadline_command gh "$@"
}

git() {
  deadline_command git "$@"
}

linear() {
  local query_file="$1"
  local variables_file="$2"
  deadline_command python3 "${LINEAR_HELPER}" \
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

orchestrator_group_alive() {
  [[ -n "${ORCHESTRATOR_PID}" ]] && kill -0 -- "-${ORCHESTRATOR_PID}" 2>/dev/null
}

cleanup() {
  local cleanup_failed=0
  set +e
  DEADLINE_ACTIVE=0
  CLEANUP_START_SECONDS="${SECONDS}"
  if [[ -n "${HERMETIC_SUPERVISOR_PID}" ]] && kill -0 "${HERMETIC_SUPERVISOR_PID}" 2>/dev/null; then
    kill -TERM "${HERMETIC_SUPERVISOR_PID}" 2>/dev/null
    wait "${HERMETIC_SUPERVISOR_PID}" 2>/dev/null
  fi
  if [[ -n "${PORT_RESERVER_PID}" ]] && kill -0 "${PORT_RESERVER_PID}" 2>/dev/null; then
    kill -TERM "${PORT_RESERVER_PID}" 2>/dev/null
    wait "${PORT_RESERVER_PID}" 2>/dev/null
  fi
  if [[ -n "${WATCHDOG_PID}" ]] && kill -0 "${WATCHDOG_PID}" 2>/dev/null; then
    kill "${WATCHDOG_PID}" 2>/dev/null
    wait "${WATCHDOG_PID}" 2>/dev/null
  fi
  if orchestrator_group_alive; then
    kill -TERM -- "-${ORCHESTRATOR_PID}" 2>/dev/null
    for _ in $(seq 1 20); do
      orchestrator_group_alive || break
      sleep 1
    done
    if orchestrator_group_alive; then kill -KILL -- "-${ORCHESTRATOR_PID}" 2>/dev/null; fi
  fi
  if [[ -n "${ORCHESTRATOR_PID}" ]]; then wait "${ORCHESTRATOR_PID}" 2>/dev/null; fi

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

  local remaining_local_roots=0
  local local_root
  for local_root in "${RESOURCE_DIR}/seeds" "${RUN_DIR}/state" "${RUN_DIR}/workspaces"; do
    rm -rf "${local_root}" || cleanup_failed=1
    if [[ -e "${local_root}" || -L "${local_root}" ]]; then
      remaining_local_roots=$((remaining_local_roots + 1))
      cleanup_failed=1
    fi
  done
  local remaining_processes=0
  if orchestrator_group_alive; then
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
    --argjson remaining_local_roots "${remaining_local_roots}" \
    --arg completed_at "$(date -u +%FT%TZ)" \
    '{schema_version:1,run_id:$run_id,scenario_passed:($scenario_passed == 1),cleanup_complete:($cleanup_failed == 0),remaining_processes:$remaining_processes,remaining_port_owner:$remaining_port_owner,remaining_local_roots:$remaining_local_roots,credential_copies:0,completed_at:$completed_at}' \
    > "${TEARDOWN_PATH}"
  set -e
  if (( cleanup_failed != 0 )); then
    echo "Live rollout teardown was incomplete: ${TEARDOWN_PATH}" >&2
    return 1
  fi
  if (( SCENARIO_PASSED == 1 )); then
    if ! jq --arg completed_at "$(date -u +%FT%TZ)" \
      '.result="passed" | .completed_at=$completed_at' \
      "${RUN_DIR}/release-evidence.json" > "${RUN_DIR}/release-evidence.tmp" ||
       ! mv "${RUN_DIR}/release-evidence.tmp" "${RUN_DIR}/release-evidence.json"; then
      echo "Could not publish passing release evidence after teardown." >&2
      return 1
    fi
    echo "Disposable live rollout and teardown passed."
    echo "Evidence: ${RUN_DIR}/release-evidence.json"
  fi
}
trap 'status=$?; trap - EXIT; cleanup || status=1; exit "${status}"' EXIT
trap 'exit 143' TERM
trap 'exit 130' INT

state_vars="${RUN_DIR}/team-states.json"
write_json "${state_vars}" --arg id "${OPENSYMPHONY_LIVE_LINEAR_TEAM_ID}" '{id:$id}'
state_result="$(linear team_states.graphql "${state_vars}")"
state_id() {
  local name="$1"
  jq -er --arg name "${name}" '.data.team.states.nodes[] | select(.name == $name) | .id' <<<"${state_result}"
}
TODO_STATE="$(state_id Todo)"
HUMAN_REVIEW_STATE="$(state_id 'Human Review')"
REWORK_STATE="$(state_id Rework)"
DONE_STATE="$(state_id Done)"

PORT_FILE="${RUN_DIR}/reserved-port"
python3 - "${PORT_FILE}" <<'PY' &
import os
from pathlib import Path
import signal
import socket
import sys

signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    port_file = Path(sys.argv[1])
    temporary = port_file.with_suffix(".tmp")
    temporary.write_text(str(sock.getsockname()[1]))
    os.replace(temporary, port_file)
    signal.pause()
PY
PORT_RESERVER_PID=$!
for _ in $(seq 1 50); do
  [[ -s "${PORT_FILE}" ]] && break
  kill -0 "${PORT_RESERVER_PID}" 2>/dev/null || break
  sleep 0.1
done
if [[ ! -s "${PORT_FILE}" ]]; then
  echo "Could not reserve a control-plane port." >&2
  exit 1
fi
PORT="$(cat "${PORT_FILE}")"
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
delivered:${alias}:${RUN_ID} and run ./scripts/complete.sh as your last action.
That script checks the edit. The rollout controller waits for your completed
turn, moves the issue to Human Review, and publishes the edit. Leave the edit
in this checkout for the controller to commit, push, and open the pull request.
Do not perform Git, GitHub, or Linear side effects.
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
  cat > "${seed}/scripts/check-integration.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
test "$(cat answer.txt)" = 'answer=42'
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
  chmod +x "${seed}/scripts/check.sh" "${seed}/scripts/check-integration.sh"
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
    description="Integrate all three terminal children. Observe alpha's seeded integration failure and request its repair without editing the initial checkout. On the active repair continuation, fix only alpha and leave beta and gamma unchanged."
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

# The controller moves a successful leaf out of the active tracker set only
# after its completed turn is durable, before the next scheduler retry tick.
for index in 0 1 2; do
  alias=(alpha beta gamma)
  seed="${RESOURCE_DIR}/seeds/${alias[index]}"
  cat > "${seed}/scripts/complete.sh" <<EOF
#!/usr/bin/env bash
set -euo pipefail
cd "\$(dirname -- "\$0")/.."
./scripts/check.sh
EOF
  chmod +x "${seed}/scripts/complete.sh"
  git -C "${seed}" add scripts/complete.sh
  git -C "${seed}" -c user.name='OpenSymphony Live Gate' -c user.email='live-gate@invalid.example' \
    commit -m 'Add leaf completion check' >/dev/null
  git -C "${seed}" push origin develop >/dev/null
done

move_issue() {
  local issue_id="$1"
  local target_state="$2"
  local vars="${RUN_DIR}/move-${issue_id}.json"
  write_json "${vars}" --arg id "${issue_id}" --arg state "${target_state}" '{id:$id,stateId:$state}'
  linear issue_move_to_state.graphql "${vars}" | jq -e '.data.issueUpdate.success == true' >/dev/null
}
update_issue_title() {
  local issue_id="$1"
  local title="$2"
  local vars="${RUN_DIR}/title-${issue_id}.json"
  write_json "${vars}" --arg id "${issue_id}" --arg title "${title}" '{id:$id,input:{title:$title}}'
  linear issue_update.graphql "${vars}" | jq -e '.data.issueUpdate.success == true' >/dev/null
}
for issue_id in "${ISSUE_IDS[@]}"; do
  move_issue "${issue_id}" "${TODO_STATE}"
done

cat > "${RUN_DIR}/integration.md" <<EOF
# Disposable integration instructions

Operate only on the three verified repository handles. In each verified
checkout, run ./scripts/check.sh and ./scripts/check-integration.sh. The latter
is the bounded parent verification command: it requires answer=42 in that
checkout. Alpha is intentionally seeded with answer=41. On the initial turn,
observe its failing integration command and request alpha through the
verification receipt's repair_repository_id without editing any checkout.
On the active repair continuation, fix only alpha and leave the verified
checkout edit for OpenSymphony to publish. Do not perform Git or provider
side effects yourself. Do not modify beta or gamma. Re-run both commands in
all three checkouts before reporting the completed repair.
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
  catalog_root: ${RUN_DIR}/state/memory
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
  poll_interval_ms: 30000
  max_turns: 8
  max_retry_backoff_ms: 10000
  stall_timeout_ms: 300000
integration:
  policy: parent_repair_prs
  use_shared_git_worktrees: true
control_plane:
  bind: 127.0.0.1:${PORT}
EOF

remaining_seconds=$((MAX_SECONDS - (SECONDS - START_SECONDS)))
if (( remaining_seconds <= 0 )); then
  echo "Live rollout deadline expired during provisioning." >&2
  exit 1
fi
OPENSYMPHONY_RELEASE_CONFIG="${CONFIG_PATH}" \
OPENSYMPHONY_HERMETIC_OUTPUT_ROOT="${RUN_DIR}/hermetic" \
  python3 - "${remaining_seconds}" "${ROOT_DIR}/scripts/hermetic_multi_repo_lifecycle.sh" "${LOG_DIR}/hermetic.log" <<'PY' &
import os
import signal
import subprocess
import sys

def interrupted(signum, _frame):
    raise SystemExit(128 + signum)

signal.signal(signal.SIGINT, interrupted)
signal.signal(signal.SIGTERM, interrupted)
with open(sys.argv[3], "wb") as log:
    gate = subprocess.Popen([sys.argv[2]], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    try:
        result = gate.wait(timeout=int(sys.argv[1]))
    except subprocess.TimeoutExpired:
        print("Hermetic gate exceeded the overall live rollout deadline.", file=sys.stderr)
        sys.exit(124)
    finally:
        if gate.poll() is None:
            try:
                os.killpg(gate.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                gate.wait(timeout=10)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(gate.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                gate.wait()
    sys.exit(result)
PY
HERMETIC_SUPERVISOR_PID=$!
wait "${HERMETIC_SUPERVISOR_PID}"
HERMETIC_SUPERVISOR_PID=""

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

if ! kill -0 "${PORT_RESERVER_PID}" 2>/dev/null; then
  echo "The reserved control-plane port was lost before launch." >&2
  exit 1
fi
kill -TERM "${PORT_RESERVER_PID}"
wait "${PORT_RESERVER_PID}"
PORT_RESERVER_PID=""
remaining_seconds=$((MAX_SECONDS - (SECONDS - START_SECONDS)))
if (( remaining_seconds <= 0 )); then
  echo "Live rollout deadline expired before orchestrator start." >&2
  exit 1
fi
python3 -c 'import os,sys; os.setsid(); os.execvp(sys.argv[1], sys.argv[1:])' \
  cargo run -- run --config "${CONFIG_PATH}" >"${LOG_DIR}/orchestrator.log" 2>&1 &
ORCHESTRATOR_PID=$!
for _ in $(seq 1 50); do
  orchestrator_group_alive && break
  kill -0 "${ORCHESTRATOR_PID}" 2>/dev/null || break
  sleep 0.1
done
if ! orchestrator_group_alive; then
  echo "The orchestrator process group did not start." >&2
  exit 1
fi
python3 - "${remaining_seconds}" "${ORCHESTRATOR_PID}" "${LOG_DIR}/watchdog.log" <<'PY' &
import os
import signal
import sys
import time

time.sleep(int(sys.argv[1]))
try:
    os.killpg(int(sys.argv[2]), 0)
except ProcessLookupError:
    sys.exit(0)
with open(sys.argv[3], "a") as log:
    log.write("Live rollout exceeded its deadline.\n")
try:
    os.killpg(int(sys.argv[2]), signal.SIGTERM)
except ProcessLookupError:
    pass
PY
WATCHDOG_PID=$!

attach_pr() {
  local issue_id="$1"
  local url="$2"
  local title="$3"
  local vars="${RUN_DIR}/attach-${issue_id}.json"
  write_json "${vars}" --arg issue "${issue_id}" --arg url "${url}" --arg title "${title}" '{issueId:$issue,url:$url,title:$title}'
  linear attachment_link_github_pr.graphql "${vars}" | jq -e '.data.attachmentLinkGitHubPR.success == true' >/dev/null
}

pr_by_branch() {
  local repository="$1"
  local branch="$2"
  gh api "repos/${repository}/pulls?state=all&head=${OPENSYMPHONY_LIVE_GITHUB_OWNER}:${branch}&base=develop&per_page=100" |
    jq -cer --arg branch "${branch}" '
      ([.[] | select(.head.ref == $branch and .base.ref == "develop")][0] // empty)
      | {number,url:.html_url,title,headRefOid:.head.sha,
         state:(if .merged_at then "MERGED" elif .state == "open" then "OPEN" else "CLOSED" end)}'
}

child_continuation_ready() {
  local identifier="$1"
  deadline_command curl --silent --show-error --fail --max-time 5 \
    "http://127.0.0.1:${PORT}/api/v1/snapshot" |
    jq -e --arg identifier "${identifier}" '
      .snapshot.issues | any(.[];
        .identifier == $identifier and
        (.tracker_state == "Todo" or .tracker_state == "Rework") and
        .runtime_state == "retry_queued" and .last_outcome == "continued")' >/dev/null
}

publish_child_if_ready() {
  local index="$1"
  local alias=(alpha beta gamma)
  local repository="${REPOSITORIES[index]}"
  local branch="feat/${SLUG}-${alias[index]}"
  local checkout candidate publisher
  checkout=""
  for candidate in "${RUN_DIR}/workspaces/${CHILD_IDENTIFIERS[index]}-"*--*; do
    [[ -d "${candidate}/.git" && ! -L "${candidate}" && ! -L "${candidate}/.git" ]] || continue
    [[ -z "${checkout}" ]] || { echo "Multiple retained child checkouts for ${CHILD_IDENTIFIERS[index]}" >&2; return 1; }
    checkout="${candidate}"
  done
  [[ -n "${checkout}" ]] || return 0
  if (( CHILD_REVIEW_TRANSITIONED[index] == 0 )); then
    jq -e --arg id "${CHILD_IDS[index]}" '.issue_id == $id and .status == "succeeded"' \
      "${checkout}/.opensymphony/run.json" >/dev/null 2>&1 || return 0
    child_continuation_ready "${CHILD_IDENTIFIERS[index]}" || return 0
  fi
  [[ -f "${checkout}/delivery.txt" && ! -L "${checkout}/delivery.txt" ]] || return 0
  [[ "$(cat "${checkout}/delivery.txt")" == "delivered:${alias[index]}:${RUN_ID}" ]] || return 0
  if (( index == 0 && ALPHA_REWORK_REQUIRED == 1 )); then
    [[ -f "${checkout}/reviewed.txt" && ! -L "${checkout}/reviewed.txt" ]] || return 0
    [[ "$(cat "${checkout}/reviewed.txt" 2>/dev/null || true)" == "reviewed:${RUN_ID}" ]] || return 0
  fi
  if (( CHILD_REVIEW_TRANSITIONED[index] == 0 )); then
    move_issue "${CHILD_IDS[index]}" "${HUMAN_REVIEW_STATE}"
    CHILD_REVIEW_TRANSITIONED[index]=1
  fi
  if (( index != 0 || ALPHA_REWORK_REQUIRED == 0 )) &&
    pr_by_branch "${repository}" "${branch}" >/dev/null 2>&1; then
    return 0
  fi
  publisher="${RESOURCE_DIR}/seeds/publisher-${alias[index]}"
  rm -rf "${publisher}"
  git clone --quiet "https://github.com/${repository}.git" "${publisher}"
  if git -C "${publisher}" ls-remote --exit-code --heads origin "${branch}" >/dev/null; then
    git -C "${publisher}" fetch origin "refs/heads/${branch}" >/dev/null
    git -C "${publisher}" checkout -B "${branch}" FETCH_HEAD >/dev/null
  else
    git -C "${publisher}" checkout -b "${branch}" origin/develop >/dev/null
  fi
  rm -f "${publisher}/delivery.txt" "${publisher}/reviewed.txt"
  cp "${checkout}/delivery.txt" "${publisher}/delivery.txt"
  if [[ -f "${checkout}/reviewed.txt" && ! -L "${checkout}/reviewed.txt" ]]; then
    cp "${checkout}/reviewed.txt" "${publisher}/reviewed.txt"
  fi
  git -C "${publisher}" add -- delivery.txt
  if [[ -f "${publisher}/reviewed.txt" ]]; then git -C "${publisher}" add -- reviewed.txt; fi
  if ! git -C "${publisher}" diff --cached --quiet; then
    git -C "${publisher}" -c core.hooksPath=/dev/null \
      -c user.name='OpenSymphony Live Gate' -c user.email='live-gate@invalid.example' \
      commit -m "Complete ${alias[index]} disposable task" >/dev/null
  fi
  git -C "${publisher}" -c core.hooksPath=/dev/null push \
    "https://github.com/${repository}.git" "HEAD:refs/heads/${branch}" >/dev/null
  if ! pr_by_branch "${repository}" "${branch}" >/dev/null 2>&1; then
    gh api -X POST "repos/${repository}/pulls" -f base=develop -f head="${branch}" \
      -f title="${SLUG}-${alias[index]} disposable delivery" \
      -f body="Disposable isolated lifecycle fixture for ${CHILD_IDENTIFIERS[index]}." >/dev/null ||
      pr_by_branch "${repository}" "${branch}" >/dev/null
  fi
}

checks_are_green() {
  local repository="$1"
  local number="$2"
  local sha
  sha="$(gh api "repos/${repository}/pulls/${number}" --jq .head.sha)"
  gh api "repos/${repository}/commits/${sha}/check-runs?per_page=100" | jq -r '
    (.check_runs | length) > 0 and all(.check_runs[];
      .status == "completed" and (.conclusion == "success" or .conclusion == "neutral" or .conclusion == "skipped"))'
}

checks_have_failed() {
  local repository="$1"
  local number="$2"
  local sha
  sha="$(gh api "repos/${repository}/pulls/${number}" --jq .head.sha)"
  gh api "repos/${repository}/commits/${sha}/check-runs?per_page=100" | jq -r '
    any(.check_runs[]; .conclusion == "failure" or .conclusion == "timed_out" or .conclusion == "action_required")'
}

declare -a CHILD_ATTACHED=(0 0 0)
declare -a CHILD_MERGED=(0 0 0)
declare -a CHILD_REVIEW_TRANSITIONED=(0 0 0)
ALPHA_REWORK_REQUIRED=0
ALPHA_FAILED_SHA=""
PARENT_ATTACHED=0
PARENT_DONE=0
PARENT_MERGED=0

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

managed_workspaces_remaining() {
  local root="${RUN_DIR}/workspaces"
  if [[ -L "${root}" ]]; then printf '%s\n' "${root}"; return; fi
  if [[ -e "${root}" && ! -d "${root}" ]]; then printf '%s\n' "${root}"; return; fi
  [[ -d "${root}" ]] || return 0
  find "${root}" -mindepth 1 -maxdepth 1 \
    ! -name '.opensymphony-orchestrator-state.json' \
    ! \( -name '.opensymphony-staging' -type d \) \
    ! \( -name parents -type d \) -print || printf 'scan-error:%s\n' "${root}"
  if [[ -L "${root}/.opensymphony-orchestrator-state.json" ]]; then
    printf '%s\n' "${root}/.opensymphony-orchestrator-state.json"
  fi
  if [[ -d "${root}/.opensymphony-staging" && ! -L "${root}/.opensymphony-staging" ]]; then
    find "${root}/.opensymphony-staging" -mindepth 1 -print || printf 'scan-error:%s\n' "${root}/.opensymphony-staging"
  fi
  if [[ -d "${root}/parents" && ! -L "${root}/parents" ]]; then
    find "${root}/parents" -mindepth 1 -maxdepth 1 ! -type d -print || printf 'scan-error:%s\n' "${root}/parents"
    find "${root}/parents" -mindepth 2 -print || printf 'scan-error:%s\n' "${root}/parents"
  fi
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
    if ! pr="$(pr_by_branch "${repository}" "${branch}" 2>/dev/null)"; then
      continue
    fi
    pr_number="$(jq -er .number <<<"${pr}")"
    pr_url="$(jq -er .url <<<"${pr}")"
    pr_title="$(jq -er .title <<<"${pr}")"
    pr_sha="$(jq -er .headRefOid <<<"${pr}")"
    if (( CHILD_ATTACHED[index] == 0 )); then
      attach_pr "${CHILD_IDS[index]}" "${pr_url}" "${pr_title}"
      CHILD_ATTACHED[index]=1
    fi

    if (( index == 0 && ALPHA_REWORK_REQUIRED == 0 )); then
      if [[ "$(checks_have_failed "${repository}" "${pr_number}")" != "true" ]]; then
        continue
      fi
      ALPHA_REWORK_REQUIRED=1
      ALPHA_FAILED_SHA="${pr_sha}"
      update_issue_title "${CHILD_IDS[index]}" "${SLUG}-alpha: CI failed; create reviewed.txt containing reviewed:${RUN_ID}"
      move_issue "${CHILD_IDS[index]}" "${REWORK_STATE}"
      CHILD_REVIEW_TRANSITIONED[index]=0
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
    gh api -X PUT "repos/${repository}/pulls/${pr_number}/merge" -f merge_method=squash |
      jq -e '.merged == true' >/dev/null
    gh api -X DELETE "repos/${repository}/git/refs/heads/${branch}" >/dev/null 2>&1 || true
    move_issue "${CHILD_IDS[index]}" "${DONE_STATE}"
    CHILD_MERGED[index]=1
  done

  if (( CHILD_MERGED[0] == 1 && CHILD_MERGED[1] == 1 && CHILD_MERGED[2] == 1 && PARENT_MERGED == 0 )); then
    repository="${REPOSITORIES[0]}"
    pr="$(gh api "repos/${repository}/pulls?state=all&base=develop&per_page=100" | jq -c '
      [.[] | select(.head.ref | startswith("fix/")) |
        {number,url:.html_url,title,headRefName:.head.ref,
         state:(if .merged_at then "MERGED" elif .state == "open" then "OPEN" else "CLOSED" end)}][0]')"
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
      if (( SECONDS - START_SECONDS >= MAX_SECONDS )); then break; fi
      if [[ -z "$(managed_workspaces_remaining)" ]]; then
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

if [[ -n "$(managed_workspaces_remaining)" ]]; then
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

if [[ -n "$(git status --porcelain)" || "$(git rev-parse HEAD)" != "${COMMIT_SHA}" || "$(shasum -a 256 "${CONFIG_PATH}" | awk '{print $1}')" != "${CONFIG_SHA}" ]]; then
  echo "Candidate checkout or selected config changed during the live rollout." >&2
  exit 1
fi

SCENARIO_PASSED=1
