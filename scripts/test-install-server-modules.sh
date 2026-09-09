#!/bin/sh
# A checksum failure must happen before any downloaded helper code is sourced.
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/nodelite-installer-modules.XXXXXX")
trap 'rm -rf "$TEST_ROOT"' EXIT HUP INT TERM
mkdir -p "$TEST_ROOT/bin"
export MODULE_FIXTURES="$TEST_ROOT" MARKER="$TEST_ROOT/executed"
awk '/^mark_step "checking privileges"/ { exit } { print }' \
  "$SCRIPT_DIR/install-server.sh" >"$TEST_ROOT/functions.sh"

cat >"$TEST_ROOT/bin/curl" <<'SH'
#!/bin/sh
set -eu
output=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    --connect-timeout|--max-time|--max-filesize) shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
case "$url" in
  https://example.com/releases/v1.2.3/SHA256SUMS.txt)
    cp "$MODULE_FIXTURES/checksums" "$output" ;;
  https://example.com/releases/v1.2.3/install-server-config.sh)
    cp "$MODULE_FIXTURES/helper.sh" "$output" ;;
  *) exit 1 ;;
esac
SH
chmod +x "$TEST_ROOT/bin/curl"

cat >"$TEST_ROOT/run.sh" <<'SH'
#!/bin/sh
set -eu
# shellcheck source=/dev/null
. "$MODULE_FIXTURES/functions.sh"
BASE_URL=https://example.com/releases/v1.2.3
TMP_SHA256=$(mktemp "$MODULE_FIXTURES/manifest.XXXXXX")
TMP_CONFIG_HELPER=$(mktemp "$MODULE_FIXTURES/module.XXXXXX")
fetch_verified_script install-server-config.sh "$TMP_CONFIG_HELPER"
# shellcheck source=/dev/null
. "$TMP_CONFIG_HELPER"
SH

export PATH="$TEST_ROOT/bin:$PATH"
cat >"$TEST_ROOT/helper.sh" <<'SH'
printf 'loaded\n' >"$MARKER"
SH
digest=$(sha256sum "$TEST_ROOT/helper.sh" | sed 's/[[:space:]].*$//')
printf '%s  install-server-config.sh\n' "$digest" >"$TEST_ROOT/checksums"
sh "$TEST_ROOT/run.sh" >"$TEST_ROOT/output" 2>&1
[ -f "$MARKER" ] || { cat "$TEST_ROOT/output"; exit 1; }

reject_module() {
  rm -f "$MARKER"
  if sh "$TEST_ROOT/run.sh" >"$TEST_ROOT/output" 2>&1; then
    printf 'unexpected helper acceptance: %s\n' "$1" >&2
    exit 1
  fi
  [ ! -e "$MARKER" ] || { printf 'executed unverified helper: %s\n' "$1" >&2; exit 1; }
}

printf '%064d  install-server-config.sh\n' 0 >"$TEST_ROOT/checksums"
reject_module mismatched-digest
printf '%s  different-helper.sh\n' "$digest" >"$TEST_ROOT/checksums"
reject_module missing-entry
printf '%s  install-server-config.sh\n%s  install-server-config.sh\n' \
  "$digest" "$digest" >"$TEST_ROOT/checksums"
reject_module duplicate-entry
printf 'invalid  install-server-config.sh\n' >"$TEST_ROOT/checksums"
reject_module invalid-digest
dd if=/dev/zero of="$TEST_ROOT/helper.sh" bs=1048576 count=1 2>/dev/null
printf x >>"$TEST_ROOT/helper.sh"
digest=$(sha256sum "$TEST_ROOT/helper.sh" | sed 's/[[:space:]].*$//')
printf '%s  install-server-config.sh\n' "$digest" >"$TEST_ROOT/checksums"
reject_module oversized-helper
printf '%s\n' 'Installer helper verification: 6 cases passed'
