#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
Usage: ./scripts/publish_crates.sh [--execute] [--dry-run] [--allow-dirty] [--from <crate>] [--skip-wait]

Publishes the OpenSymphony workspace crates to crates.io in dependency order.

Modes:
  --dry-run       Run `cargo publish --dry-run` for each crate.
  --execute       Perform the real publish sequence.

Options:
  --allow-dirty   Pass `--allow-dirty` through to cargo publish.
  --from <crate>  Resume from the named crate in the publish order.
  --skip-wait     Do not wait for each uploaded crate to appear on crates.io.

Notes:
  - Choose exactly one of `--dry-run` or `--execute`.
  - A full workspace dry run cannot verify dependent crates until their
    internal dependencies already exist on crates.io. For a first release,
    dry-run the leaf crates, then use `--execute` for the staged upload.
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
from_crate=""
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
    --from)
      if (($# < 2)); then
        echo "--from requires a crate name" >&2
        usage
        exit 1
      fi
      from_crate="$2"
      shift 2
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

packages=(
  opensymphony-domain
  opensymphony-workflow
  opensymphony-workspace
  opensymphony-linear
  opensymphony-orchestrator
  opensymphony-control
  opensymphony-openhands
  opensymphony-tui
  opensymphony-testkit
  opensymphony-cli
  opensymphony
)

start_index=0
if [[ -n "$from_crate" ]]; then
  found=false
  for i in "${!packages[@]}"; do
    if [[ "${packages[$i]}" == "$from_crate" ]]; then
      start_index="$i"
      found=true
      break
    fi
  done
  if [[ "$found" != true ]]; then
    echo "crate not found in publish order: $from_crate" >&2
    exit 1
  fi
fi

version="$(workspace_version)"
if [[ -z "$version" ]]; then
  echo "failed to determine workspace version from Cargo.toml" >&2
  exit 1
fi

for ((i = start_index; i < ${#packages[@]}; i++)); do
  pkg="${packages[$i]}"
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
done
