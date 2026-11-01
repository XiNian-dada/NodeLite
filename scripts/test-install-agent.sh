#!/bin/sh
# Installer functions run on disposable paths; root Linux runs also exercise the service user.
# shellcheck disable=SC2034
set -eu
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
INSTALLER="$SCRIPT_DIR/install-agent.sh"
(
  eval "$(awk '/^cleanup\(\)/ { printing=1 } /^trap cleanup/ { exit } printing { print }' "$INSTALLER")"
  SIGNED_STAGE='' TMP_PATH='' BOOTSTRAP_TMP='' CURL_AUTH_CONFIG='' CHECKSUMS_TMP=''
  cleanup
)
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT HUP INT TERM

eval "$(awk '/^configure_agent_config_paths\(\)/ { printing=1 } /^cleanup_legacy_auto_update\(\)/ { exit } printing { print }' "$INSTALLER")"
eval "$(awk '/^write_systemd_unit\(\)/ { printing=1 } /^write_launchd_plist\(\)/ { exit } printing { print }' "$INSTALLER")"

SERVICE_KIND=systemd
TRAFFIC_CONTROL=0
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
grep -Fx 'CapabilityBoundingSet=' "$UNIT_PATH" >/dev/null
grep -Fx 'AmbientCapabilities=' "$UNIT_PATH" >/dev/null
grep -Fx 'RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6' "$UNIT_PATH" >/dev/null
TRAFFIC_CONTROL=1
write_systemd_unit
grep -Fx 'User='"$SERVICE_USER" "$UNIT_PATH" >/dev/null
grep -Fx 'CapabilityBoundingSet=CAP_NET_ADMIN' "$UNIT_PATH" >/dev/null
grep -Fx 'AmbientCapabilities=CAP_NET_ADMIN' "$UNIT_PATH" >/dev/null
grep -Fx 'RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK' "$UNIT_PATH" >/dev/null

refuse_signed_updater_override
cp "$UNIT_PATH" "$TEMP_DIR/original-unit"
printf '%s\n' 'ExecStart=/var/lib/nodelite-updater/code/active --config /var/lib/nodelite-agent/agent.toml' >"$UNIT_PATH"
# shellcheck disable=SC2317
fail() { printf '%s\n' "$*" >&2; return 1; }
if refuse_signed_updater_override >/dev/null 2>&1; then
  printf '%s\n' 'Legacy installation must not bypass enrolled signature policy' >&2
  exit 1
fi
cp "$TEMP_DIR/original-unit" "$UNIT_PATH"

BOOTSTRAP_TMP="$TEMP_DIR/bootstrap.toml"
printf '%s\n' 'replacement-test-fixture' >"$BOOTSTRAP_TMP"
config_refreshed=1
install_agent_config
cmp "$BOOTSTRAP_TMP" "$CONFIG_PATH"

fail() { printf '%s\n' "$*" >&2; return 1; }
TMP_PATH="$TEMP_DIR/candidate-agent"
install -m 0600 "$SCRIPT_DIR/fixtures/preflight-agent.sh" "$TMP_PATH"
PREFLIGHT_EXPECTED_CONFIG="$CONFIG_PATH"
PREFLIGHT_TEST_REJECT=0
export PREFLIGHT_EXPECTED_CONFIG PREFLIGHT_TEST_REJECT
config_refreshed=0
check_saved_credentials
[ -x "$TMP_PATH" ]
mv "$CONFIG_PATH" "$TEMP_DIR/saved-active.toml"
PREFLIGHT_EXPECTED_CONFIG="$LEGACY_CONFIG_PATH"
check_saved_credentials
mv "$TEMP_DIR/saved-active.toml" "$CONFIG_PATH"
config_refreshed=1
PREFLIGHT_EXPECTED_CONFIG="$BOOTSTRAP_TMP"
check_saved_credentials
PREFLIGHT_TEST_REJECT=1
if check_saved_credentials >/dev/null 2>&1; then
  printf '%s\n' 'Invalid credentials must abort the upgrade' >&2
  exit 1
fi
cmp "$BOOTSTRAP_TMP" "$CONFIG_PATH"
awk '
  /^check_saved_credentials$/ { checked=1 }
  /systemctl stop nodelite-agent.service/ || /^install .*\$TMP_PATH.*\$BIN_PATH/ {
    if (!checked) exit 1
  }
' "$INSTALLER"

SHA256_X86_64=pinned-x86 SHA256_AARCH64=pinned-arm
select_release_target amd64
[ "$TARGET" = x86_64-unknown-linux-musl ] && [ "$EXPECTED_SHA256" = pinned-x86 ]
select_release_target aarch64
[ "$TARGET" = aarch64-unknown-linux-musl ] && [ "$EXPECTED_SHA256" = pinned-arm ]
if (select_release_target riscv64) >/dev/null 2>&1; then
  printf '%s\n' 'Unsupported architectures must be rejected' >&2
  exit 1
fi

# Server-rendered mirror pins must not block the signed path on enrolled Agents.
awk '
  /enrolled Agent ignores --base-url\/--sha256-\* overrides/ { warned=1 }
  /installer overrides are not allowed/ { if (!warned) exit 1; found=1 }
  /^  \[ "\$MODE" != install \] && \[ -z "\$BOOTSTRAP_URL\$BINARY_URL" \]/ { guard=1 }
  END { exit !(found && guard) }
' "$INSTALLER"

SERVICE_KIND=launchd
# Linux musl pins must not be applied to macOS artifacts.
select_release_target x86_64
[ "$TARGET" = x86_64-apple-darwin ] && [ -z "$EXPECTED_SHA256" ]
select_release_target arm64
[ "$TARGET" = aarch64-apple-darwin ] && [ -z "$EXPECTED_SHA256" ]
configure_agent_config_paths
[ "$CONFIG_PATH" = "$CONFIG_DIR/agent.toml" ]
printf '%s\n' 'Agent config migration, permissions and upgrade preflight: ok'
