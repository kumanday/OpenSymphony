#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
Usage: ./scripts/publish_crates.sh [--execute] [--dry-run] [--allow-dirty] [--skip-wait]

Publishes the public OpenSymphony package to crates.io.

Modes:
  --dry-run       Run `cargo publish --dry-run -p opensymphony`.
  --execute       Perform the real publish for `opensymphony`.

Options:
  --allow-dirty   Pass `--allow-dirty` through to cargo publish.
  --skip-wait     Do not wait for the uploaded package to appear on crates.io.

Notes:
  - Choose exactly one of `--dry-run` or `--execute`.
  - OpenSymphony now publishes a single crates.io package: `opensymphony`.
  - Internal subsystem boundaries remain in-repo source trees under `crates/`,
    but they are not published as standalone crates.
EOF
}

workspace_version() {
  awk -F'"' '
    /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
    /^\[/ && !/^\[workspace\.package\]$/ { in_workspace_package = 0 }
    in_workspace_package && /^version = / { print $2; exit }
  ' Cargo.toml
}

wait_for_crate() {
  local crate="$1"
  local version="$2"
  local attempts=60

  for ((attempt = 1; attempt <= attempts; attempt++)); do
    if curl --silent --fail "https://crates.io/api/v1/crates/${crate}/${version}" >/dev/null; then
      return 0
    fi
    sleep 5
  done

  echo "timed out waiting for ${crate} ${version} to appear on crates.io" >&2
  return 1
}

mode=""
allow_dirty=false
skip_wait=false

while (($# > 0)); do
  case "$1" in
    --dry-run)
      mode="dry-run"
      shift
      ;;
    --execute)
      mode="execute"
      shift
      ;;
    --allow-dirty)
      allow_dirty=true
      shift
      ;;
    --skip-wait)
      skip_wait=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ -z "$mode" ]]; then
  echo "choose one of --dry-run or --execute" >&2
  usage
  exit 1
fi

version="$(workspace_version)"
if [[ -z "$version" ]]; then
  echo "failed to determine workspace version from Cargo.toml" >&2
  exit 1
fi

pkg="opensymphony"
cmd=(cargo publish -p "$pkg")

if [[ "$mode" == "dry-run" ]]; then
  cmd+=(--dry-run)
fi

if [[ "$allow_dirty" == true ]]; then
  cmd+=(--allow-dirty)
fi

echo "==> ${cmd[*]}"
"${cmd[@]}"

if [[ "$mode" == "execute" && "$skip_wait" != true ]]; then
  echo "==> waiting for ${pkg} ${version} to appear on crates.io"
  wait_for_crate "$pkg" "$version"
fi
