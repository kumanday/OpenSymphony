#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")"/.. && pwd)"
export OPENSYMPHONY_LIVE_OPENHANDS="${OPENSYMPHONY_LIVE_OPENHANDS:-0}"

if [[ "${OPENSYMPHONY_LIVE_OPENHANDS}" != "1" ]]; then
  echo "Set OPENSYMPHONY_LIVE_OPENHANDS=1 to run the live local suite."
  exit 1
fi

for required_var in OPENSYMPHONY_OPENHANDS_MODEL OPENSYMPHONY_OPENHANDS_API_KEY; do
  if [[ -z "${!required_var:-}" ]]; then
    echo "Missing required environment variable: ${required_var}" >&2
    exit 1
  fi
done

RUN_ID="$(date +%Y%m%d-%H%M%S)"
OUTPUT_ROOT="${OPENSYMPHONY_LIVE_SUITE_OUTPUT_ROOT:-${ROOT_DIR}/target/live-local}"
RUN_DIR="${OUTPUT_ROOT%/}/${RUN_ID}"
LOG_DIR="${RUN_DIR}/logs"
SERVER_PORT="${OPENSYMPHONY_LIVE_SUITE_SERVER_PORT:-8010}"
SERVER_BASE_URL="http://127.0.0.1:${SERVER_PORT}"

mkdir -p "${LOG_DIR}"
export OPENSYMPHONY_LIVE_SUITE_OUTPUT_DIR="${RUN_DIR}"
export OPENSYMPHONY_LIVE_SUITE_BASE_URL="${SERVER_BASE_URL}"

if [[ -z "${OPENAI_API_KEY:-}" ]]; then
  export OPENAI_API_KEY="${OPENSYMPHONY_OPENHANDS_API_KEY}"
fi

cleanup() {
  if [[ -n "${DOCTOR_PID:-}" ]] && kill -0 "${DOCTOR_PID}" 2>/dev/null; then
    kill "${DOCTOR_PID}" 2>/dev/null || true
    wait "${DOCTOR_PID}" 2>/dev/null || true
  fi

  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "${SERVER_PID}" 2>/dev/null; then
    kill "${SERVER_PID}" 2>/dev/null || true
    wait "${SERVER_PID}" 2>/dev/null || true
  fi
}

kill_doctor_server_residue() {
  while read -r pid; do
    if [[ -n "${pid}" ]]; then
      kill "${pid}" 2>/dev/null || true
      wait "${pid}" 2>/dev/null || true
    fi
  done < <(pgrep -f 'openhands.agent_server --host 127.0.0.1 --port 8000' || true)
}

run_doctor_preflight() {
  local log_file="${LOG_DIR}/doctor.log"
  : > "${log_file}"

  cargo run -p opensymphony-cli -- doctor --config examples/configs/local-dev.with-live-openhands.yaml --live-openhands \
    2>&1 | tee "${log_file}" &
  DOCTOR_PID=$!

  local stop_seen=0
  local stop_deadline=0

  while kill -0 "${DOCTOR_PID}" 2>/dev/null; do
    if grep -q '\[PASS\] openhands-supervisor-stop:' "${log_file}"; then
      if (( stop_seen == 0 )); then
        stop_seen=1
        stop_deadline=$((SECONDS + 15))
      elif (( SECONDS >= stop_deadline )); then
        echo "Doctor shutdown watchdog: forcing the pinned local server to exit after supervisor-stop." >&2
        kill_doctor_server_residue
        break
      fi
    fi

    sleep 1
  done

  if kill -0 "${DOCTOR_PID}" 2>/dev/null; then
    for _ in $(seq 1 20); do
      if ! kill -0 "${DOCTOR_PID}" 2>/dev/null; then
        break
      fi
      sleep 1
    done
  fi

  if kill -0 "${DOCTOR_PID}" 2>/dev/null; then
    echo "Doctor command did not exit after the shutdown watchdog ran. See ${log_file}." >&2
    kill "${DOCTOR_PID}" 2>/dev/null || true
  fi

  wait "${DOCTOR_PID}"
  DOCTOR_PID=""
}

wait_for_server_ready() {
  for _ in $(seq 1 120); do
    if curl -fsS "${SERVER_BASE_URL}/openapi.json" >/dev/null; then
      return 0
    fi

    if [[ -n "${SERVER_PID:-}" ]] && ! kill -0 "${SERVER_PID}" 2>/dev/null; then
      echo "Pinned OpenHands server exited before readiness. See ${LOG_DIR}/agent-server.stderr.log." >&2
      return 1
    fi

    sleep 1
  done

  echo "Pinned OpenHands server did not become ready at ${SERVER_BASE_URL}." >&2
  return 1
}

trap cleanup EXIT

cd "${ROOT_DIR}"

echo "Live local suite artifacts: ${RUN_DIR}"

run_doctor_preflight

OPENHANDS_SERVER_PORT="${SERVER_PORT}" tools/openhands-server/run-local.sh \
  >"${LOG_DIR}/agent-server.stdout.log" \
  2>"${LOG_DIR}/agent-server.stderr.log" &
SERVER_PID=$!
echo "${SERVER_PID}" > "${RUN_DIR}/agent-server.pid"

wait_for_server_ready

cargo test -p opensymphony-openhands --test live_local_suite -- --ignored --nocapture --test-threads=1 \
  2>&1 | tee "${LOG_DIR}/live-suite.log"

cat > "${RUN_DIR}/README.txt" <<EOF
OpenSymphony live local suite artifacts

doctor log: ${LOG_DIR}/doctor.log
agent-server stdout: ${LOG_DIR}/agent-server.stdout.log
agent-server stderr: ${LOG_DIR}/agent-server.stderr.log
live suite log: ${LOG_DIR}/live-suite.log
lifecycle summary: ${RUN_DIR}/lifecycle/summary.json
reconnect summary: ${RUN_DIR}/reconnect/summary.json
EOF

echo "Live local suite completed successfully."
echo "Artifact index: ${RUN_DIR}/README.txt"
