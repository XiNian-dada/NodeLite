#!/bin/sh
# Exercise the generated official unit without changing the host's routing or qdiscs.
# shellcheck disable=SC2034
set -eu
[ "$(id -u)" -eq 0 ] || { echo 'Run as root on Linux with systemd' >&2; exit 1; }
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
TEST_DIR=$(mktemp -d /run/nodelite-tc-test.XXXXXX)
TEST_NS="nodelite-tc-$$"
UNIT_NAME="nodelite-tc-test-$$.service"
UNIT_PATH="/run/systemd/system/$UNIT_NAME"
cleanup() {
  systemctl stop "$UNIT_NAME" >/dev/null 2>&1 || true
  rm -f "$UNIT_PATH"
  systemctl daemon-reload
  ip netns del "$TEST_NS" || true
  rm -rf "$TEST_DIR"
}
trap cleanup EXIT HUP INT TERM
chmod 0755 "$TEST_DIR"
install -m 0755 "$1" "$TEST_DIR/tests"
ip netns add "$TEST_NS"
ip -n "$TEST_NS" link add nltest0 type dummy
ip -n "$TEST_NS" link set nltest0 up
ip -n "$TEST_NS" address add 192.0.2.1/24 dev nltest0
ip -n "$TEST_NS" route add default dev nltest0
ip netns exec "$TEST_NS" tc qdisc add dev nltest0 clsact
for direction in ingress egress; do
  ip netns exec "$TEST_NS" tc filter add dev nltest0 "$direction" pref 42 protocol all matchall action pass
done
eval "$(awk '/^write_systemd_unit\(\)/ { printing=1 } /^write_launchd_plist\(\)/ { exit } printing { print }' "$SCRIPT_DIR/install-agent.sh")"
SERVICE_USER=nobody
SERVICE_GROUP=$(id -gn nobody)
STATE_DIR="$TEST_DIR/state"
mkdir "$STATE_DIR"
chown "$SERVICE_USER:$SERVICE_GROUP" "$STATE_DIR"
chmod 0700 "$STATE_DIR"
BIN_PATH="$TEST_DIR/tests"
CONFIG_PATH=unused
for TRAFFIC_CONTROL in 1 0; do
  write_systemd_unit
  # The test uses the installer sandbox verbatim; only its command/lifetime and test namespace differ.
  if [ "$TRAFFIC_CONTROL" = 1 ]; then
    test_case=applies_changes_and_removes_only_owned_filters
  else
    test_case=disabled_unit_has_no_network_admin_capability
  fi
  cat >>"$UNIT_PATH" <<EOF

[Service]
Type=oneshot
Restart=no
ExecStart=
ExecStart=$BIN_PATH traffic_control::linux_system_tests::$test_case --ignored --exact --nocapture
Environment=NODELITE_TC_SYSTEM_TEST=1
NetworkNamespacePath=/run/netns/$TEST_NS
EOF
  systemctl daemon-reload
  if ! systemctl start "$UNIT_NAME"; then
    journalctl -u "$UNIT_NAME" --no-pager
    exit 1
  fi
  [ "$(systemctl show "$UNIT_NAME" -p ExecMainStatus --value)" = 0 ]
done
printf '%s\n' 'Real systemd traffic control: apply, change, clear, foreign filters and disabled sandbox passed'
