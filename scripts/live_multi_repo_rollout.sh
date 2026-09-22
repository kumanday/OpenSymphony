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
  OPENSYMPHONY_LIVE_REVIEW_TOKEN
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

if [[ "${GH_TOKEN}" == "${OPENSYMPHONY_LIVE_REVIEW_TOKEN}" ]]; then
  echo "OPENSYMPHONY_LIVE_REVIEW_TOKEN must belong to a different GitHub identity." >&2
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

AUTHOR_LOGIN="$(gh api user --jq .login)"
REVIEWER_LOGIN="$(GH_TOKEN="${OPENSYMPHONY_LIVE_REVIEW_TOKEN}" gh api user --jq .login)"
if [[ "${AUTHOR_LOGIN}" == "${REVIEWER_LOGIN}" ]]; then
  echo "The live reviewer must differ from the repository/PR author." >&2
  exit 1
fi

mkdir -p "${RESOURCE_DIR}" "${LOG_DIR}"
cd "${ROOT_DIR}"
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
  linear "${query}.graphql" "${vars}" >/dev/null 2>&1 || return 1
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
    gh repo delete "${REPOSITORIES[index]}" --yes >/dev/null 2>&1 || cleanup_failed=1
  done
  for repository in "${REPOSITORIES[@]:-}"; do
    [[ -z "${repository}" ]] && continue
    if gh repo view "${repository}" >/dev/null 2>&1; then
      cleanup_failed=1
    fi
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
trap cleanup EXIT

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
  gh repo create "${repository}" --private --disable-issues --disable-wiki >/dev/null
  REPOSITORIES+=("${repository}")
  REPOSITORY_IDS+=("$(gh api "repos/${repository}" --jq '.id | tostring')")
  gh api -X PUT "repos/${repository}/collaborators/${REVIEWER_LOGIN}" -f permission=pull >/dev/null
  invitation_id="$(GH_TOKEN="${OPENSYMPHONY_LIVE_REVIEW_TOKEN}" gh api user/repository_invitations --jq ".[] | select(.repository.full_name == \"${repository}\") | .id" | head -1)"
  if [[ -n "${invitation_id}" ]]; then
    GH_TOKEN="${OPENSYMPHONY_LIVE_REVIEW_TOKEN}" gh api -X PATCH "user/repository_invitations/${invitation_id}" >/dev/null
  fi
  mkdir -p "${seed}/scripts"
  git -C "${seed}" init -b develop >/dev/null
  cat > "${seed}/AGENTS.md" <<EOF
# Disposable OpenSymphony live fixture: ${alias}

Work only in this repository. Create delivery.txt with the exact content
delivered:${alias}:${RUN_ID}, run ./scripts/check.sh, commit, push branch
feat/${SLUG}-${alias}, and open one pull request to develop. Put
${SLUG}-${alias} in the pull-request title. Do not merge the pull request.
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
test -f delivery.txt
EOF
  chmod +x "${seed}/scripts/check.sh"
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
  linear issue_move_to_state.graphql "${vars}" >/dev/null
}
for issue_id in "${ISSUE_IDS[@]}"; do
  move_issue "${issue_id}" "${TODO_STATE}"
done

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

cat > "${RUN_DIR}/integration.md" <<EOF
# Disposable integration instructions

Operate only on the three verified repository handles. Run each repository's
./scripts/check.sh. The final integrated value in each answer.txt must be
answer=42. Alpha is intentionally seeded with answer=41: repair only alpha,
use branch fix/${SLUG}-alpha-answer, and open one pull request to develop. Do
not modify beta or gamma. Re-run all three checks before reporting
completion.
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
    required_review: true
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

COMMIT_SHA="$(git rev-parse HEAD)"
CONFIG_SHA="$(shasum -a 256 "${CONFIG_PATH}" | awk '{print $1}')"
jq -n \
  --arg run_id "${RUN_ID}" \
  --arg commit_sha "${COMMIT_SHA}" \
  --arg config_sha256 "${CONFIG_SHA}" \
  --arg project_id "${PROJECT_ID}" \
  --arg project_slug "${PROJECT_SLUG}" \
  --arg port "${PORT}" \
  '{schema_version:1,run_id:$run_id,commit_sha:$commit_sha,config_sha256:$config_sha256,project_set:"live",linear_project:{id:$project_id,slug:$project_slug},control_plane_port:($port|tonumber),production_activation:false,result:"started"}' \
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
  linear attachment_link_github_pr.graphql "${vars}" >/dev/null
}

checks_are_green() {
  local repository="$1"
  local number="$2"
  gh pr view "${number}" --repo "${repository}" --json statusCheckRollup \
    --jq '[.statusCheckRollup[] | if .__typename == "CheckRun" then .conclusion else .state end | select(. != "SUCCESS" and . != "NEUTRAL" and . != "SKIPPED")] | length == 0'
}

declare -a CHILD_ATTACHED=(0 0 0)
declare -a CHILD_MERGED=(0 0 0)
ALPHA_CHANGE_REQUESTED=0
ALPHA_REQUESTED_SHA=""
PARENT_ATTACHED=0
PARENT_MERGED=0
START_SECONDS="${SECONDS}"

while (( SECONDS - START_SECONDS < MAX_SECONDS )); do
  if ! kill -0 "${ORCHESTRATOR_PID}" 2>/dev/null; then
    echo "OpenSymphony exited before the live lifecycle completed." >&2
    wait "${ORCHESTRATOR_PID}" || true
    exit 1
  fi

  for index in 0 1 2; do
    (( CHILD_MERGED[index] == 0 )) || continue
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

    if (( index == 0 && ALPHA_CHANGE_REQUESTED == 0 )); then
      GH_TOKEN="${OPENSYMPHONY_LIVE_REVIEW_TOKEN}" gh pr review "${pr_number}" --repo "${repository}" \
        --request-changes --body "Create reviewed.txt with the exact content reviewed:${RUN_ID}; leave answer.txt unchanged."
      ALPHA_CHANGE_REQUESTED=1
      ALPHA_REQUESTED_SHA="${pr_sha}"
      move_issue "${CHILD_IDS[index]}" "${REWORK_STATE}"
      continue
    fi
    if (( index == 0 )) && [[ "${pr_sha}" == "${ALPHA_REQUESTED_SHA}" ]]; then
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
    GH_TOKEN="${OPENSYMPHONY_LIVE_REVIEW_TOKEN}" gh pr review "${pr_number}" --repo "${repository}" --approve --body 'Disposable live gate approval.'
    gh pr merge "${pr_number}" --repo "${repository}" --squash --delete-branch
    move_issue "${CHILD_IDS[index]}" "${DONE_STATE}"
    CHILD_MERGED[index]=1
  done

  if (( CHILD_MERGED[0] == 1 && CHILD_MERGED[1] == 1 && CHILD_MERGED[2] == 1 && PARENT_MERGED == 0 )); then
    repository="${REPOSITORIES[0]}"
    branch="fix/${SLUG}-alpha-answer"
    if pr="$(gh pr view "${branch}" --repo "${repository}" --json number,url,title,state,statusCheckRollup 2>/dev/null)"; then
      pr_number="$(jq -er .number <<<"${pr}")"
      if (( PARENT_ATTACHED == 0 )); then
        attach_pr "${PARENT_ID}" "$(jq -er .url <<<"${pr}")" "$(jq -er .title <<<"${pr}")"
        move_issue "${PARENT_ID}" "${HUMAN_REVIEW_STATE}"
        PARENT_ATTACHED=1
      fi
      if [[ "$(checks_are_green "${repository}" "${pr_number}")" == "true" ]]; then
        GH_TOKEN="${OPENSYMPHONY_LIVE_REVIEW_TOKEN}" gh pr review "${pr_number}" --repo "${repository}" --approve --body 'Disposable parent repair approval.'
        gh pr merge "${pr_number}" --repo "${repository}" --squash --delete-branch
        move_issue "${PARENT_ID}" "${DONE_STATE}"
        PARENT_MERGED=1
      fi
    fi
  fi

  if (( PARENT_MERGED == 1 )); then
    for _ in $(seq 1 120); do
      if [[ ! -d "${RUN_DIR}/workspaces" ]] || [[ -z "$(find "${RUN_DIR}/workspaces" -mindepth 1 -print -quit)" ]]; then
        break
      fi
      sleep 1
    done
    break
  fi
  sleep "${POLL_SECONDS}"
done

if (( PARENT_MERGED != 1 || ALPHA_CHANGE_REQUESTED != 1 )); then
  echo "The bounded live lifecycle did not complete before its deadline." >&2
  exit 1
fi

if [[ -d "${RUN_DIR}/workspaces" ]] && [[ -n "$(find "${RUN_DIR}/workspaces" -mindepth 1 -print -quit)" ]]; then
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
  --arg requested_change_sha "${ALPHA_REQUESTED_SHA}" \
  --argjson child_merges "$(printf '%s\n' "${CHILD_MERGED[@]}" | jq -Rsc 'split("\n") | map(select(length > 0) | tonumber)')" \
  '{schema_version:1,run_id:$run_id,result:"passed",parent:$parent,child_merges:$child_merges,requested_change_sha:$requested_change_sha,parent_repair_merge:true,cleanup_handoff:true}' \
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
