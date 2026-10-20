#!/bin/sh
# shellcheck disable=SC2015,SC2154
# Sourced only after version-bound checksum verification by install-agent.sh.

prepare_signed_update() {
  [ "$SERVICE_KIND" = systemd ] && [ "$TRAFFIC_CONTROL" = 0 ] &&
    [ "$INSTALL_DIR" = /usr/local/bin ] && [ "$STATE_DIR" = /var/lib/nodelite-agent ] &&
    [ "$CONFIG_DIR" = /etc/nodelite ] || fail 'signed updates require the default capability-free Linux installation'
  [ -f /sys/fs/cgroup/cgroup.controllers ] && [ ! -e /.dockerenv ] &&
    [ ! -e /run/systemd/container ] || fail 'signed updates require non-container systemd/cgroup-v2 Linux'
  [ -z "$BINARY_URL$SHA256_X86_64$SHA256_AARCH64" ] || fail 'signed enrollment cannot use binary overrides'
  [ -z "$BOOTSTRAP_URL" ] || [ "$existing_install" -eq 0 ] ||
    fail 'signed enrollment cannot replace existing credentials or use binary overrides'
  if [ ! -x /usr/bin/python3 ]; then
    printf '%s\n' 'Installing the OS Python 3 runtime for migration/recovery (no pip packages).'
    if command -v apt-get >/dev/null 2>&1; then
      apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends python3 || fail 'OS runtime installation failed'
    elif command -v dnf >/dev/null 2>&1; then
      dnf install -y python3 || fail 'OS runtime installation failed'
    else
      fail 'this OS cannot provision the recovery runtime automatically; manual upgrades remain available'
    fi
  fi
  /usr/bin/python3 -I -c 'import sys; assert sys.version_info >= (3, 9)' || fail 'signed migration needs OS Python >= 3.9'
  # A world-writable ancestor is unsuitable for privileged recovery source.
  /usr/bin/python3 -I -c '
import os, pathlib, stat
path = pathlib.Path("/usr/local/lib")
for item in (path, *path.parents):
    info = item.lstat()
    assert stat.S_ISDIR(info.st_mode) and info.st_uid == 0 and not info.st_mode & 0o022
'
  SIGNED_STAGE="$(mktemp -d /usr/local/lib/nodelite-enroll.XXXXXX)"
  enrollment_asset="nodelite-enrollment-$TARGET.tar.gz"
  EXPECTED_SHA256=''
  fetch_expected_sha256 "$enrollment_asset"
  curl -q -fsSL --connect-timeout 15 --max-time 180 --max-filesize 68157440 \
    --proto '=https,http' --proto-redir '=https' "$BASE_URL/$enrollment_asset" -o "$SIGNED_STAGE/package.tar.gz" ||
    fail 'this release has no available signed enrollment package; the existing Agent was not changed'
  [ "$(calculate_sha256 "$SIGNED_STAGE/package.tar.gz")" = "$EXPECTED_SHA256" ] || fail 'enrollment package checksum mismatch'
  /usr/bin/python3 -I - "$SIGNED_STAGE" <<'PY'
import pathlib, sys, tarfile
stage = pathlib.Path(sys.argv[1])
sources = {"tools/__init__.py", "tools/nodelite_release/__init__.py", "tools/nodelite_release/io.py"}
sources.update("tools/nodelite_migrate/" + name + ".py" for name in
               ("__init__", "__main__", "common", "bundle", "transaction", "audit", "installer"))
units = {"nodelite-agent.service", "nodelite-agent-preflight.service"}
units.update("nodelite-updater-" + name for name in ("apply.service", "boot.service", "reconcile.service", "reconcile.timer",
             "preflight.service", "preflight.path", "stop.service", "stop.path", "restart.service", "restart.path"))
expected = sources | {"systemd/" + name for name in units} | {
    "root.json", "nodelite-agent", "nodelite-updater", "manifest.json", "enrollment.json"}
with tarfile.open(stage / "package.tar.gz", "r:gz") as archive:
    members, total = {}, 0
    for member in archive:
        limit = 33554432 if member.name in {"nodelite-agent", "nodelite-updater"} else 262144
        if member.name not in expected or member.name in members or not member.isfile() or not 0 <= member.size <= limit:
            raise ValueError("unsafe enrollment archive member")
        members[member.name] = member
        total += member.size
        if total > 68157440:
            raise ValueError("enrollment archive exceeds size bound")
    if members.keys() != expected:
        raise ValueError("incomplete enrollment archive")
    for name, member in members.items():
        path = stage / name
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        with archive.extractfile(member) as source, path.open("xb") as destination:
            destination.write(source.read())
PY
  /usr/bin/python3 -I -c '
import pathlib, sys
sys.path.insert(0, sys.argv[1])
from tools.nodelite_migrate.installer import validate
validate(pathlib.Path(sys.argv[1]), sys.argv[2])
' "$SIGNED_STAGE" "$TARGET"
}

finish_signed_update() {
  /usr/bin/python3 -I -c '
import sys
sys.path.insert(0, sys.argv.pop(1))
from tools.nodelite_migrate.__main__ import main
main()
' "$SIGNED_STAGE" --enroll --bundle "$SIGNED_STAGE" --target "$TARGET" ||
    fail 'signed migration did not commit; recovery preserves the previous Agent and current credentials'
  printf '%s\n' 'Signed updates enabled. Future upgrades use the same install-agent.sh --mode upgrade command.'
}
