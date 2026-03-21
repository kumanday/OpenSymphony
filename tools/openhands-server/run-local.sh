#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
pyproject="${script_dir}/pyproject.toml"
lockfile="${script_dir}/uv.lock"
version="$(tr -d '[:space:]' < "${script_dir}/version.txt")"
placeholder_requirement='openhands-agent-server-placeholder==0+bootstrap.placeholder'

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

echo "Launching pinned OpenHands agent-server ${version} from ${script_dir}." >&2
exec uv run --directory "${script_dir}" --locked --extra agent-server --module openhands.agent_server "$@"
