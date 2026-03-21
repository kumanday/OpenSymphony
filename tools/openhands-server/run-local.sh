#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
version="$(tr -d '[:space:]' < "${script_dir}/version.txt")"

if [[ "${version}" == "TODO-openhands-sdk-version" ]]; then
  echo "OpenHands agent-server version is not pinned yet." >&2
  echo "Update ${script_dir}/version.txt and ${script_dir}/pyproject.toml before running." >&2
  exit 1
fi

echo "Bootstrap placeholder only." >&2
echo "Resolve ${script_dir}/uv.lock for version ${version} before launching the local server." >&2
exit 1
