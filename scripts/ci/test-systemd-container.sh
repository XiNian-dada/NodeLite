#!/bin/bash
# The dedicated job has its own cgroup and PID namespaces inside Runner DinD.
set -euo pipefail
binary=$(realpath "$1")
root=$(cd "$(dirname "$0")/../.." && pwd)
report="${RUNNER_TEMP:-/tmp}/systemd-report"
mkdir -p "$report"
[ "$(id -u)" -eq 0 ]
[ -f /.dockerenv ] || { echo 'Run only in the dedicated disposable Docker job' >&2; exit 1; }
export NODELITE_TEST_ROOT="$root" NODELITE_TEST_BINARY="$binary" NODELITE_TEST_REPORT="$report"
cat >/etc/systemd/system/nodelite-ci.service <<'UNIT'
[Unit]
Description=NodeLite disposable systemd acceptance
After=systemd-journald.service
[Service]
Type=oneshot
PassEnvironment=NODELITE_TEST_ROOT NODELITE_TEST_BINARY NODELITE_TEST_REPORT
ExecStart=/bin/bash /usr/local/bin/nodelite-systemd-tests
StandardOutput=journal+console
StandardError=journal+console
UNIT
cat >/etc/systemd/system/nodelite-ci.target <<'UNIT'
[Unit]
Description=NodeLite acceptance target
Wants=nodelite-ci.service
After=nodelite-ci.service
UNIT
cat >/usr/local/bin/nodelite-systemd-tests <<'SCRIPT'
#!/bin/bash
set -euo pipefail
export NO_PROXY=localhost,127.0.0.1
export no_proxy=localhost,127.0.0.1
finish() {
  result=$?
  journalctl --sync
  journalctl --no-pager >"$NODELITE_TEST_REPORT/journal.log"
  printf '%s\n' "$result" >"$NODELITE_TEST_REPORT/exit-code"
  systemctl --no-block exit "$result"
}
trap finish EXIT
cd "$NODELITE_TEST_ROOT"
python3 scripts/test-install-server-upgrade.py --systemd crash not_ready wrong_version success migrate_fail corrupt_backup restore_fail signal
sh scripts/test-traffic-control-systemd.sh "$NODELITE_TEST_BINARY"
SCRIPT
chmod 0755 /usr/local/bin/nodelite-systemd-tests
# A private /run keeps systemd's sockets and service state out of the Runner process.
set +e
timeout --signal=TERM --kill-after=10s 600s \
  unshare --mount --pid --fork --mount-proc bash -c '
    mount --make-rprivate /
    mount -o remount,rw /sys/fs/cgroup
    mount -t tmpfs tmpfs /run
    mkdir -p /run/lock
    export container=docker
    exec /lib/systemd/systemd --system --unit=nodelite-ci.target --log-target=console --log-level=warning
  ' >"$report/systemd.log" 2>&1
status=$?
set -e
cat "$report/systemd.log"
cat "$report/journal.log" 2>/dev/null || true
[ "$status" -eq 0 ] && [ "$(cat "$report/exit-code" 2>/dev/null)" = 0 ] || exit 1
printf '%s\n' 'Real systemd upgrades and traffic-control sandbox passed in Docker'
