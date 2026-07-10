#!/bin/sh
set -eu

assets_dir="${1:-release-assets}"
assets_parent="$(dirname -- "$assets_dir")"
assets_name="$(basename -- "$assets_dir")"

server_installer="$assets_name/release-scripts/install-server.sh"
agent_installer="$assets_name/release-scripts/install-agent.sh"
script_checksums="$assets_name/SHA256SUMS-scripts.txt"
merged_checksums="$assets_name/SHA256SUMS.txt"

(
  cd "$assets_parent"
  [ -f "$server_installer" ] || {
    printf '%s\n' "missing release asset: $server_installer" >&2
    exit 1
  }
  [ -f "$agent_installer" ] || {
    printf '%s\n' "missing release asset: $agent_installer" >&2
    exit 1
  }

  sha256sum "$server_installer" "$agent_installer" >"$script_checksums"
  find "$assets_name" -type f -name 'SHA256SUMS-*.txt' | sort | while IFS= read -r checksum_file; do
    cat "$checksum_file"
  done >"$merged_checksums"
)
