#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
BOOTSTRAP="$SCRIPT_DIR/server-web-update.sh"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nodelite-web-update-test.XXXXXX")"
FAKE_BIN="$TEST_ROOT/bin"
FIXTURES="$TEST_ROOT/fixtures"
LOG_PATH="$TEST_ROOT/update.log"
CACHE_DIR="$TEST_ROOT/cache"
MARKER_PATH="$TEST_ROOT/installer-ran"
CURL_LOG_PATH="$TEST_ROOT/curl.log"
REPOSITORY="https://github.com/example/NodeLite"

cleanup() {
  rm -rf "$TEST_ROOT"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$FAKE_BIN" "$FIXTURES"

cat >"$FIXTURES/install-server.sh" <<'EOF'
#!/bin/sh
set -eu
printf '%s\n%s\n%s\n' \
  "$NODELITE_SERVER_VERSION" \
  "$NODELITE_SERVER_BASE_URL" \
  "$NODELITE_SERVER_MODE" >"$TEST_MARKER_PATH"
exit "${FAKE_INSTALLER_EXIT:-0}"
EOF
cp "$FIXTURES/install-server.sh" "$FIXTURES/install-server.original.sh"
installer_sha256="$(sha256sum "$FIXTURES/install-server.sh" | sed 's/[[:space:]].*$//')"
printf '%s  %s\n' "$installer_sha256" "release-assets/release-scripts/install-server.sh" \
  >"$FIXTURES/SHA256SUMS.txt"

cat >"$FAKE_BIN/curl" <<'EOF'
#!/bin/sh
set -eu

output=""
write_effective_url=0
url=""
printf '%s\n' "$*" >>"$FAKE_CURL_LOG"
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      output="$2"
      shift 2
      ;;
    -w)
      write_effective_url=1
      shift 2
      ;;
    --proto|--proto-redir|--connect-timeout|--max-time|--max-filesize)
      shift 2
      ;;
    -*)
      shift
      ;;
    *)
      url="$1"
      shift
      ;;
  esac
done

if [ -n "${FAKE_CURL_SIGNAL:-}" ]; then
  case "$url" in
    */releases/latest)
      trap 'exit 0' HUP INT TERM
      parent_pid="$(ps -o ppid= -p "$$" | tr -d '[:space:]')"
      kill "-$FAKE_CURL_SIGNAL" "$parent_pid"
      while :; do
        sleep 1
      done
      ;;
  esac
fi

case "$url" in
  */releases/latest)
    [ "$write_effective_url" -eq 1 ] || exit 1
    printf '%s' "$FAKE_REDIRECT_URL"
    ;;
  */releases/download/v1.2.3/SHA256SUMS.txt)
    cp "$FAKE_FIXTURES/SHA256SUMS.txt" "$output"
    ;;
  */releases/download/v1.2.3/install-server.sh)
    cp "$FAKE_FIXTURES/install-server.sh" "$output"
    ;;
  *)
    printf '%s\n' "unexpected curl URL: $url" >&2
    exit 1
    ;;
esac
EOF
chmod 0700 "$FAKE_BIN/curl"

export FAKE_FIXTURES="$FIXTURES"
export FAKE_REDIRECT_URL="$REPOSITORY/releases/tag/v1.2.3"
export FAKE_CURL_LOG="$CURL_LOG_PATH"
export FAKE_INSTALLER_EXIT=0
export TEST_MARKER_PATH="$MARKER_PATH"

run_bootstrap() {
  PATH="$FAKE_BIN:$PATH" \
  NODELITE_UPDATE_LOG="$LOG_PATH" \
  NODELITE_UPDATE_CACHE_DIR="$CACHE_DIR" \
  NODELITE_UPDATE_REPOSITORY="$REPOSITORY" \
    sh "$BOOTSTRAP"
}

capture_bootstrap_status() {
  set +e
  run_bootstrap
  bootstrap_status="$?"
  set -e
}

assert_status() {
  expected_status="$1"
  if [ "$bootstrap_status" -ne "$expected_status" ]; then
    printf '%s\n' "expected bootstrap status $expected_status, got $bootstrap_status" >&2
    exit 1
  fi
}

assert_installer_did_not_run() {
  [ ! -e "$MARKER_PATH" ] || {
    printf '%s\n' "installer ran before bootstrap validation completed" >&2
    exit 1
  }
}

run_bootstrap
sed -n '1p' "$MARKER_PATH" | grep -Fx 'v1.2.3' >/dev/null
sed -n '2p' "$MARKER_PATH" | grep -Fx "$REPOSITORY/releases/download/v1.2.3" >/dev/null
sed -n '3p' "$MARKER_PATH" | grep -Fx 'upgrade' >/dev/null
grep -F "nodelite-update: target release tag=v1.2.3" "$LOG_PATH" >/dev/null
grep -F "nodelite-update: installer sha256=$installer_sha256 verified" "$LOG_PATH" >/dev/null
if [ "$(wc -l <"$CURL_LOG_PATH" | tr -d '[:space:]')" -ne 3 ]; then
  printf '%s\n' "expected exactly three curl calls" >&2
  exit 1
fi
if grep -v -- '--proto =https --proto-redir =https --connect-timeout 10' \
  "$CURL_LOG_PATH" >/dev/null; then
  printf '%s\n' "curl call omitted required HTTPS restrictions or connect timeout" >&2
  exit 1
fi
grep -F -- '--max-time 30' "$CURL_LOG_PATH" >/dev/null
grep -F -- '--max-time 60 --max-filesize 1048576' "$CURL_LOG_PATH" >/dev/null

rm -f "$MARKER_PATH"
printf '%064d  install-server.sh\n' 0 >"$FIXTURES/SHA256SUMS.txt"
capture_bootstrap_status
assert_status 1
assert_installer_did_not_run
grep -F "nodelite-update: error: downloaded installer checksum mismatch" "$LOG_PATH" >/dev/null

printf '%s  install-server.sh\n%s  install-server.sh\n' \
  "$installer_sha256" "$installer_sha256" >"$FIXTURES/SHA256SUMS.txt"
capture_bootstrap_status
assert_status 1
assert_installer_did_not_run
grep -F "nodelite-update: error: release checksums contain an invalid installer digest" \
  "$LOG_PATH" >/dev/null

printf '%s  other-script.sh\n' "$installer_sha256" >"$FIXTURES/SHA256SUMS.txt"
capture_bootstrap_status
assert_status 1
assert_installer_did_not_run
grep -F "nodelite-update: error: release checksums contain an invalid installer digest" \
  "$LOG_PATH" >/dev/null

dd if=/dev/zero of="$FIXTURES/install-server.sh" bs=1048576 count=1 2>/dev/null
printf 'x' >>"$FIXTURES/install-server.sh"
oversized_sha256="$(sha256sum "$FIXTURES/install-server.sh" | sed 's/[[:space:]].*$//')"
printf '%s  install-server.sh\n' "$oversized_sha256" >"$FIXTURES/SHA256SUMS.txt"
capture_bootstrap_status
assert_status 1
assert_installer_did_not_run
grep -F "nodelite-update: error: downloaded installer exceeds size limit" "$LOG_PATH" >/dev/null
cp "$FIXTURES/install-server.original.sh" "$FIXTURES/install-server.sh"

printf '%s  %s\n' "$installer_sha256" "install-server.sh" >"$FIXTURES/SHA256SUMS.txt"
FAKE_REDIRECT_URL="$REPOSITORY/releases/tag/v1.2.3-rc.1"
export FAKE_REDIRECT_URL
capture_bootstrap_status
assert_status 1
assert_installer_did_not_run
grep -F "nodelite-update: error: resolved release tag is not a stable version tag" \
  "$LOG_PATH" >/dev/null

FAKE_REDIRECT_URL="https://github.com/attacker/NodeLite/releases/tag/v1.2.3"
export FAKE_REDIRECT_URL
capture_bootstrap_status
assert_status 1
assert_installer_did_not_run
grep -F "nodelite-update: error: latest release redirected outside the configured repository" \
  "$LOG_PATH" >/dev/null

FAKE_REDIRECT_URL="$REPOSITORY/releases/tag/v1.2.3"
FAKE_INSTALLER_EXIT=42
export FAKE_REDIRECT_URL FAKE_INSTALLER_EXIT
capture_bootstrap_status
assert_status 42
grep -F "nodelite-update: finished exit=42" "$LOG_PATH" >/dev/null

FAKE_INSTALLER_EXIT=0
export FAKE_INSTALLER_EXIT
for signal_case in HUP:129 INT:130 TERM:143; do
  signal_name="${signal_case%:*}"
  signal_status="${signal_case#*:}"
  rm -f "$MARKER_PATH"
  FAKE_CURL_SIGNAL="$signal_name"
  export FAKE_CURL_SIGNAL
  capture_bootstrap_status
  assert_status "$signal_status"
  assert_installer_did_not_run
  grep -F "nodelite-update: interrupted signal=$signal_name" "$LOG_PATH" >/dev/null
  grep -F "nodelite-update: finished exit=$signal_status" "$LOG_PATH" >/dev/null
  if find "$CACHE_DIR" -type f -print -quit | grep . >/dev/null; then
    printf '%s\n' "temporary update files remained after $signal_name" >&2
    exit 1
  fi
done
unset FAKE_CURL_SIGNAL
