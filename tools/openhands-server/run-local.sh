#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
pyproject="${script_dir}/pyproject.toml"
lockfile="${script_dir}/uv.lock"
version="$(tr -d '[:space:]' < "${script_dir}/version.txt")"
placeholder_requirement='openhands-agent-server-placeholder==0+bootstrap.placeholder'
runtime_sandbox="process"
server_host="127.0.0.1"
server_port="${OPENHANDS_SERVER_PORT:-8000}"

if [[ ! -f "${pyproject}" ]]; then
  echo "OpenHands agent-server pyproject is missing." >&2
  echo "Restore ${pyproject} before running." >&2
  exit 1
fi

if [[ ! -f "${lockfile}" ]]; then
  echo "OpenHands agent-server lockfile is missing." >&2
  echo "Restore ${lockfile} before running." >&2
  exit 1
fi

if [[ "${version}" == "0+bootstrap.placeholder" ]]; then
  echo "OpenHands agent-server version is not pinned yet." >&2
  echo "Update ${script_dir}/version.txt and ${script_dir}/pyproject.toml before running." >&2
  exit 1
fi

if grep -Fq "${placeholder_requirement}" "${pyproject}"; then
  echo "OpenHands agent-server package metadata is still unresolved." >&2
  echo "Update ${pyproject} with the pinned agent-server dependency before running." >&2
  exit 1
fi

if grep -Fq "Placeholder bootstrap file." "${lockfile}"; then
  echo "OpenHands agent-server lockfile is not resolved yet." >&2
  echo "Replace ${lockfile} with a resolved uv lock output before running." >&2
  exit 1
fi

if ! [[ "${server_port}" =~ ^[0-9]+$ ]] || (( 10#${server_port} < 1 || 10#${server_port} > 65535 )); then
  echo "OpenHands agent-server port must be an integer between 1 and 65535." >&2
  echo "Set OPENHANDS_SERVER_PORT to a valid loopback port before running." >&2
  exit 1
fi

if (( $# > 0 )); then
  echo "run-local.sh does not accept extra agent-server CLI flags in supervised mode." >&2
  echo "Use OPENHANDS_SERVER_PORT to change the local port if needed." >&2
  exit 1
fi

export RUNTIME="${runtime_sandbox}"

echo "Launching pinned OpenHands agent-server ${version} from ${script_dir} on ${server_host}:${server_port} with RUNTIME=${RUNTIME}." >&2
exec uv run \
  --directory "${script_dir}" \
  --locked \
  --extra agent-server \
  --module openhands.agent_server \
  --host "${server_host}" \
  --port "${server_port}"
