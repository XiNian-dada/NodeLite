#!/bin/sh
# Web settings update bootstrap. This file is embedded in the server binary at
# compile time, so the downloaded installer is never trusted before its release
# checksum has been verified.

set -u
umask 077

: "${NODELITE_UPDATE_LOG:?NODELITE_UPDATE_LOG is required}"
: "${NODELITE_UPDATE_CACHE_DIR:?NODELITE_UPDATE_CACHE_DIR is required}"
: "${NODELITE_UPDATE_REPOSITORY:?NODELITE_UPDATE_REPOSITORY is required}"

log="$NODELITE_UPDATE_LOG"
cache_dir="$NODELITE_UPDATE_CACHE_DIR"
repository="${NODELITE_UPDATE_REPOSITORY%/}"
tmp_script=""
tmp_checksums=""
tmp_resolved_url=""
active_pid=""

CURL_CONNECT_TIMEOUT_SECS=10
CURL_RESOLVE_TIMEOUT_SECS=30
CURL_DOWNLOAD_TIMEOUT_SECS=60
CHECKSUM_MAX_BYTES=1048576
INSTALLER_MAX_BYTES=1048576

# shellcheck disable=SC2317 # Invoked indirectly by the EXIT trap.
cleanup() {
  [ -n "$tmp_script" ] && rm -f "$tmp_script"
  [ -n "$tmp_checksums" ] && rm -f "$tmp_checksums"
  [ -n "$tmp_resolved_url" ] && rm -f "$tmp_resolved_url"
  return 0
}

finish() {
  status="$1"
  printf '%s\n' "nodelite-update: finished exit=$status at $(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$log"
  exit "$status"
}

fail_update() {
  printf '%s\n' "nodelite-update: error: $1" >>"$log"
  finish 1
}

# shellcheck disable=SC2317 # Invoked indirectly by the signal traps.
stop_active_process() {
  if [ -n "$active_pid" ]; then
    kill -TERM "$active_pid" 2>/dev/null || true
    wait "$active_pid" 2>/dev/null || true
    active_pid=""
  fi
}

# shellcheck disable=SC2317 # Invoked indirectly by the signal traps.
handle_signal() {
  signal_name="$1"
  signal_status="$2"
  stop_active_process
  printf '%s\n' "nodelite-update: interrupted signal=$signal_name" >>"$log"
  finish "$signal_status"
}

run_secure_curl() {
  curl \
    --proto '=https' \
    --proto-redir '=https' \
    --connect-timeout "$CURL_CONNECT_TIMEOUT_SECS" \
    "$@" &
  active_pid="$!"
  wait "$active_pid"
  command_status="$?"
  active_pid=""
  return "$command_status"
}

calculate_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | sed 's/[[:space:]].*$//'
    return 0
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | sed 's/[[:space:]].*$//'
    return 0
  fi
  return 1
}

trap cleanup EXIT
trap 'handle_signal HUP 129' HUP
trap 'handle_signal INT 130' INT
trap 'handle_signal TERM 143' TERM

: >"$log" || exit 1
printf '%s\n' "nodelite-update: started at $(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$log"
mkdir -p "$cache_dir" || fail_update "failed to create private update cache"
chmod 0700 "$cache_dir" || fail_update "failed to secure private update cache"
tmp_resolved_url="$(mktemp "$cache_dir/release-url.XXXXXX")" \
  || fail_update "failed to create temporary release URL file"
chmod 0600 "$tmp_resolved_url" \
  || fail_update "failed to secure temporary release URL file"

case "$repository" in
  https://github.com/*/*)
    ;;
  *)
    fail_update "release repository must be an https://github.com owner/repository URL"
    ;;
esac

release_url="$repository/releases/latest"
printf '%s\n' "nodelite-update: resolving stable release from $release_url" >>"$log"
if ! run_secure_curl \
  --max-time "$CURL_RESOLVE_TIMEOUT_SECS" \
  -fsSIL \
  -o /dev/null \
  -w '%{url_effective}' \
  "$release_url" >"$tmp_resolved_url"; then
  fail_update "failed to resolve latest stable release"
fi
resolved_url="$(cat "$tmp_resolved_url")"

tag_prefix="$repository/releases/tag/"
case "$resolved_url" in
  "$tag_prefix"*)
    target_tag="${resolved_url#"$tag_prefix"}"
    ;;
  *)
    fail_update "latest release redirected outside the configured repository"
    ;;
esac
case "$target_tag" in
  ""|*-*|*[!A-Za-z0-9._+]*)
    fail_update "resolved release tag is not a stable version tag"
    ;;
esac

asset_base_url="$repository/releases/download/$target_tag"
installer_url="$asset_base_url/install-server.sh"
checksums_url="$asset_base_url/SHA256SUMS.txt"
printf '%s\n' "nodelite-update: target release tag=$target_tag" >>"$log"
printf '%s\n' "nodelite-update: installer url=$installer_url" >>"$log"

tmp_script="$(mktemp "$cache_dir/install-server.XXXXXX")" \
  || fail_update "failed to create temporary installer file"
tmp_checksums="$(mktemp "$cache_dir/SHA256SUMS.XXXXXX")" \
  || fail_update "failed to create temporary checksum file"
chmod 0600 "$tmp_script" "$tmp_checksums" \
  || fail_update "failed to secure temporary update files"

if ! run_secure_curl \
  --max-time "$CURL_DOWNLOAD_TIMEOUT_SECS" \
  --max-filesize "$CHECKSUM_MAX_BYTES" \
  -fsSL \
  "$checksums_url" \
  -o "$tmp_checksums" >>"$log" 2>&1; then
  fail_update "failed to download release checksums"
fi
checksum_size="$(wc -c <"$tmp_checksums" | tr -d '[:space:]')"
if [ "$checksum_size" -gt "$CHECKSUM_MAX_BYTES" ]; then
  fail_update "release checksum manifest exceeds size limit"
fi
expected_sha256="$(awk -v artifact="install-server.sh" '
  NF >= 2 {
    path = $2
    sub(/^\*/, "", path)
    count = split(path, parts, "/")
    if (parts[count] == artifact) {
      print $1
    }
  }
' "$tmp_checksums")"
case "$expected_sha256" in
  ""|*[!0-9A-Fa-f]*)
    fail_update "release checksums contain an invalid installer digest"
    ;;
esac
if [ "${#expected_sha256}" -ne 64 ]; then
  fail_update "release checksums contain an invalid installer digest"
fi
expected_sha256="$(printf '%s' "$expected_sha256" | tr 'A-F' 'a-f')"

if ! run_secure_curl \
  --max-time "$CURL_DOWNLOAD_TIMEOUT_SECS" \
  --max-filesize "$INSTALLER_MAX_BYTES" \
  -fsSL \
  "$installer_url" \
  -o "$tmp_script" >>"$log" 2>&1; then
  fail_update "failed to download versioned installer"
fi
installer_size="$(wc -c <"$tmp_script" | tr -d '[:space:]')"
if [ "$installer_size" -gt "$INSTALLER_MAX_BYTES" ]; then
  fail_update "downloaded installer exceeds size limit"
fi
if ! actual_sha256="$(calculate_sha256 "$tmp_script")"; then
  fail_update "missing required command: sha256sum or shasum"
fi
if [ "$actual_sha256" != "$expected_sha256" ]; then
  fail_update "downloaded installer checksum mismatch"
fi
printf '%s\n' "nodelite-update: installer sha256=$actual_sha256 verified" >>"$log"

chmod 0700 "$tmp_script" || fail_update "failed to make verified installer executable"
printf '%s\n' "nodelite-update: running verified installer in upgrade mode" >>"$log"
NODELITE_SERVER_VERSION="$target_tag" \
NODELITE_SERVER_BASE_URL="$asset_base_url" \
NODELITE_SERVER_MODE=upgrade \
  sh "$tmp_script" >>"$log" 2>&1 &
active_pid="$!"
wait "$active_pid"
update_status="$?"
active_pid=""
finish "$update_status"
