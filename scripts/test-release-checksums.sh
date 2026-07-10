#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
MERGE_SCRIPT="$SCRIPT_DIR/merge-release-checksums.sh"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nodelite-release-checksums-test.XXXXXX")"
ASSETS_DIR="$TEST_ROOT/release-assets"

cleanup() {
  rm -rf "$TEST_ROOT"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$ASSETS_DIR/release-scripts" "$ASSETS_DIR/release-test"
cp "$SCRIPT_DIR/install-server.sh" "$ASSETS_DIR/release-scripts/install-server.sh"
cp "$SCRIPT_DIR/install-agent.sh" "$ASSETS_DIR/release-scripts/install-agent.sh"
printf '%064d  release-assets/release-test/nodelite-server-test\n' 0 \
  >"$ASSETS_DIR/release-test/SHA256SUMS-test.txt"

(
  cd "$TEST_ROOT"
  sh "$MERGE_SCRIPT" release-assets
)

server_sha256="$(sha256sum "$ASSETS_DIR/release-scripts/install-server.sh" | sed 's/[[:space:]].*$//')"
agent_sha256="$(sha256sum "$ASSETS_DIR/release-scripts/install-agent.sh" | sed 's/[[:space:]].*$//')"
grep -Fx "$server_sha256  release-assets/release-scripts/install-server.sh" \
  "$ASSETS_DIR/SHA256SUMS.txt" >/dev/null
grep -Fx "$agent_sha256  release-assets/release-scripts/install-agent.sh" \
  "$ASSETS_DIR/SHA256SUMS.txt" >/dev/null
grep -Fx "$(printf '%064d' 0)  release-assets/release-test/nodelite-server-test" \
  "$ASSETS_DIR/SHA256SUMS.txt" >/dev/null

if [ "$(grep -Ec '^[0-9a-f]{64}  release-assets/release-scripts/install-(server|agent)\.sh$' "$ASSETS_DIR/SHA256SUMS.txt")" -ne 2 ]; then
  printf '%s\n' "release installer checksum entries used an unexpected path or format" >&2
  exit 1
fi
