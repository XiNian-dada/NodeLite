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
EOF
installer_sha256="$(sha256sum "$FIXTURES/install-server.sh" | sed 's/[[:space:]].*$//')"
printf '%s  %s\n' "$installer_sha256" "release-assets/release-scripts/install-server.sh" \
  >"$FIXTURES/SHA256SUMS.txt"

cat >"$FAKE_BIN/curl" <<'EOF'
#!/bin/sh
set -eu

output=""
write_effective_url=0
url=""
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
    -*)
      shift
      ;;
    *)
      url="$1"
      shift
      ;;
  esac
done

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
export TEST_MARKER_PATH="$MARKER_PATH"

run_bootstrap() {
  PATH="$FAKE_BIN:$PATH" \
  NODELITE_UPDATE_LOG="$LOG_PATH" \
  NODELITE_UPDATE_CACHE_DIR="$CACHE_DIR" \
  NODELITE_UPDATE_REPOSITORY="$REPOSITORY" \
    sh "$BOOTSTRAP"
}

run_bootstrap
sed -n '1p' "$MARKER_PATH" | grep -Fx 'v1.2.3' >/dev/null
sed -n '2p' "$MARKER_PATH" | grep -Fx "$REPOSITORY/releases/download/v1.2.3" >/dev/null
sed -n '3p' "$MARKER_PATH" | grep -Fx 'upgrade' >/dev/null
grep -F "nodelite-update: target release tag=v1.2.3" "$LOG_PATH" >/dev/null
grep -F "nodelite-update: installer sha256=$installer_sha256 verified" "$LOG_PATH" >/dev/null

rm -f "$MARKER_PATH"
printf '%064d  install-server.sh\n' 0 >"$FIXTURES/SHA256SUMS.txt"
if run_bootstrap; then
  printf '%s\n' "checksum mismatch unexpectedly succeeded" >&2
  exit 1
fi
[ ! -e "$MARKER_PATH" ] || {
  printf '%s\n' "installer ran after checksum mismatch" >&2
  exit 1
}
grep -F "nodelite-update: error: downloaded installer checksum mismatch" "$LOG_PATH" >/dev/null

printf '%s  %s\n' "$installer_sha256" "install-server.sh" >"$FIXTURES/SHA256SUMS.txt"
FAKE_REDIRECT_URL="$REPOSITORY/releases/tag/v1.2.3-rc.1"
export FAKE_REDIRECT_URL
if run_bootstrap; then
  printf '%s\n' "pre-release redirect unexpectedly succeeded" >&2
  exit 1
fi
grep -F "nodelite-update: error: resolved release tag is not a stable version tag" \
  "$LOG_PATH" >/dev/null

FAKE_REDIRECT_URL="https://github.com/attacker/NodeLite/releases/tag/v1.2.3"
export FAKE_REDIRECT_URL
if run_bootstrap; then
  printf '%s\n' "cross-repository redirect unexpectedly succeeded" >&2
  exit 1
fi
grep -F "nodelite-update: error: latest release redirected outside the configured repository" \
  "$LOG_PATH" >/dev/null
