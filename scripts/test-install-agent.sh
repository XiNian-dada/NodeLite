#!/bin/sh
# Installer functions run on disposable paths; root Linux runs also exercise the service user.
# shellcheck disable=SC2034
set -eu
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
INSTALLER="$SCRIPT_DIR/install-agent.sh"
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT HUP INT TERM

eval "$(awk '/^configure_agent_config_paths\(\)/ { printing=1 } /^cleanup_legacy_auto_update\(\)/ { exit } printing { print }' "$INSTALLER")"
eval "$(awk '/^write_systemd_unit\(\)/ { printing=1 } /^write_launchd_plist\(\)/ { exit } printing { print }' "$INSTALLER")"

SERVICE_KIND=systemd
SERVICE_USER=$(id -un)
SERVICE_GROUP=$(id -gn)
if [ "$(id -u)" -eq 0 ] && [ "$(uname -s)" = Linux ]; then
  SERVICE_USER=nobody
  SERVICE_GROUP=$(id -gn nobody)
  chmod 0711 "$TEMP_DIR"
else
  # Ownership itself is checked by the root Linux CI run, not faked as a local assertion.
  chown() { :; }
fi
INSTALL_DIR="$TEMP_DIR/bin"
CONFIG_DIR="$TEMP_DIR/etc"
STATE_DIR="$TEMP_DIR/state"
UNIT_PATH="$TEMP_DIR/agent.service"
BIN_PATH="$INSTALL_DIR/nodelite-agent"
configure_agent_config_paths
prepare_directories
[ "$CONFIG_PATH" = "$STATE_DIR/agent.toml" ]
printf '%s\n' '[agent]' 'token = "legacy-test-fixture"' >"$LEGACY_CONFIG_PATH"
config_refreshed=0
install_agent_config
cmp "$LEGACY_CONFIG_PATH" "$CONFIG_PATH"

if [ "$(id -u)" -eq 0 ] && [ "$(uname -s)" = Linux ]; then
  [ "$(stat -c %a "$STATE_DIR")" = 700 ]
  [ "$(stat -c %a "$CONFIG_PATH")" = 600 ]
  [ "$(stat -c %U "$STATE_DIR")" = "$SERVICE_USER" ]
  [ "$(stat -c %U "$CONFIG_PATH")" = "$SERVICE_USER" ]
  # Expansion belongs to the service user's shell.
  # shellcheck disable=SC2016
  runuser -u "$SERVICE_USER" -- sh -c '
    umask 077
    printf "%s\n" "refreshed-test-fixture" >"$1.tmp"
    mv "$1.tmp" "$1"
  ' sh "$CONFIG_PATH"
else
  printf '%s\n' 'refreshed-test-fixture' >"$CONFIG_PATH"
fi
install_agent_config
grep -Fx 'refreshed-test-fixture' "$CONFIG_PATH" >/dev/null
write_systemd_unit
grep -Fx "ExecStart=$BIN_PATH --config $STATE_DIR/agent.toml" "$UNIT_PATH" >/dev/null
grep -Fx 'ProtectSystem=strict' "$UNIT_PATH" >/dev/null
grep -Fx "ReadWritePaths=$STATE_DIR" "$UNIT_PATH" >/dev/null

BOOTSTRAP_TMP="$TEMP_DIR/bootstrap.toml"
printf '%s\n' 'replacement-test-fixture' >"$BOOTSTRAP_TMP"
config_refreshed=1
install_agent_config
cmp "$BOOTSTRAP_TMP" "$CONFIG_PATH"

SERVICE_KIND=launchd
configure_agent_config_paths
[ "$CONFIG_PATH" = "$CONFIG_DIR/agent.toml" ]
printf '%s\n' 'Agent config migration and permissions: ok'
