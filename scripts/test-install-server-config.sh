#!/bin/sh
# Variables below are consumed by functions loaded through eval, which shellcheck cannot trace.
# shellcheck disable=SC2034
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
INSTALLER="$SCRIPT_DIR/install-server.sh"
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEMP_DIR"' EXIT HUP INT TERM

extract_config_functions() {
  awk '
    /^toml_get_raw\(\) \{/ { printing = 1 }
    printing && /^mark_step "checking privileges"/ { exit }
    printing { print }
  ' "$INSTALLER"
}

fail() {
  printf '%s\n' "$*" >&2
  exit 1
}

eval "$(extract_config_functions)"

LISTEN_HOST="127.0.0.1"
LISTEN_PORT="20000"
PUBLIC_SCHEME="https"
PUBLIC_HOST="monitor.example.com"
CONFIG_DIR="/opt/nodelite/config"
DATA_DIR="/opt/nodelite/data"
READONLY_USERNAME="viewer"
READONLY_PASSWORD="test-password"
SERVER_STALE_AFTER_SECS="20"
SERVER_PING_INTERVAL_SECS="10"
SERVER_MAX_MESSAGE_BYTES="65536"
SERVER_TOKEN_VERIFY_MAX_PARALLELISM="4"
WS_MAX_TOTAL_CONNECTIONS="1024"
WS_MAX_CONNECTIONS_PER_IP="32"
WS_AUTH_FAIL_WINDOW_SECS="300"
WS_AUTH_FAIL_MAX_ATTEMPTS="12"
WS_AUTH_BLOCK_SECS="900"
UI_REFRESH_INTERVAL_SECS="5"
GEOIP_ENABLED="true"
GEOIP_PROVIDER="ipwhois"
GEOIP_EDITION="country-lite"
GEOIP_DATABASE_PATH=""
GEOIP_AUTO_UPDATE="false"
GEOIP_UPDATE_INTERVAL_DAYS="30"
IGNORED_FILESYSTEMS_RAW='["tmpfs", "devtmpfs", "overlay"]'

fresh_config="$(render_server_config)"
printf '%s\n' "$fresh_config" |
  grep -Fx 'token_verify_max_parallelism = 4' >/dev/null

existing_config="$TEMP_DIR/existing.toml"
printf '%s\n' '[server]' 'token_verify_max_parallelism = 7' >"$existing_config"
SERVER_TOKEN_VERIFY_MAX_PARALLELISM="4"
load_existing_server_defaults "$existing_config"
[ "$SERVER_TOKEN_VERIFY_MAX_PARALLELISM" = "7" ] || {
  printf '%s\n' "upgrade defaults did not preserve token verify parallelism" >&2
  exit 1
}

missing_config="$TEMP_DIR/missing.toml"
printf '%s\n' '[server]' 'listen = "127.0.0.1:20000"' >"$missing_config"
TMP_CONFIG=""
CONFIG_DEFAULTS_ADDED=0
SERVER_TOKEN_VERIFY_MAX_PARALLELISM="4"
ensure_toml_default \
  "$missing_config" server token_verify_max_parallelism \
  "token_verify_max_parallelism = $SERVER_TOKEN_VERIFY_MAX_PARALLELISM"
grep -Fx 'token_verify_max_parallelism = 4' "$missing_config" >/dev/null
[ "$CONFIG_DEFAULTS_ADDED" -eq 1 ] || {
  printf '%s\n' "missing token verify default was not counted" >&2
  exit 1
}

ensure_toml_default \
  "$existing_config" server token_verify_max_parallelism \
  "token_verify_max_parallelism = $SERVER_TOKEN_VERIFY_MAX_PARALLELISM"
grep -Fx 'token_verify_max_parallelism = 7' "$existing_config" >/dev/null

# shellcheck disable=SC2016
grep -F \
  'ensure_toml_default "$config_path" server token_verify_max_parallelism "token_verify_max_parallelism = $SERVER_TOKEN_VERIFY_MAX_PARALLELISM"' \
  "$INSTALLER" >/dev/null
