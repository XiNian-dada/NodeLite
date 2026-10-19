"""Real default-install migration, guarded by the disposable KVM marker before root writes."""

import argparse
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import time

from cold_support import CONFIG as LAB_CONFIG, api, setup as setup_server, stop_all, wait_for
from lab_io import atomic_bytes, atomic_json, digest
from real_guest import halt
from setup_guest import directory, guard, run, unit
from test_guest import ACTIVE_UNITS, systemctl

INPUT = Path("/home/lab/enrollment-input")
PUBLIC = Path("/var/lib/nlup-enrollment-public")
OPERATOR = Path("/root/nlup-enrollment-operator")
JOURNAL = Path("/var/lib/nodelite-updater-migration/journal.json")
CONFIG = Path("/var/lib/nodelite-agent/agent.toml")
UNIT = Path("/etc/systemd/system/nodelite-agent.service")


def production():
    sys.path.insert(0, str(OPERATOR))
    from tools.nodelite_migrate import common
    return common


def migration_args(*extra):
    info = json.loads((INPUT / "fingerprints.json").read_text())
    args = ["/usr/bin/python3", "-m", "tools.nodelite_migrate", "--bundle", str(INPUT / "bundle"),
        "--bundle-sha256", info["bundle_sha256"], "--root-sha256", info["root_sha256"],
        "--metadata-url", "http://127.0.0.1:18094/metadata/", "--targets-url", "http://127.0.0.1:18094/targets/", *extra]
    return args


def migrate(*extra, check=True):
    result = subprocess.run(migration_args(*extra), cwd=OPERATOR, capture_output=True, text=True, timeout=120)
    if check and result.returncode:
        raise RuntimeError(result.stderr[-1600:])
    return result


def status():
    return api("/api/nodes/cold-node")


def online(version):
    value = status()
    return value.get("online") and value.get("identity", {}).get("agent_version") == version


def archive(path):
    path = Path(path)
    if path.exists() or path.is_symlink():
        destination = Path("/var/lib/nlup-lab") / ("enrollment-archive-" + secrets.token_hex(8))
        path.rename(destination)


def setup():
    guard()
    common = production()
    systemctl("stop", *common.UNITS, common.RECOVERY, "nlup-lab-enrollment.service", "nlup-lab-enrollment-mirror.service", check=False)
    systemctl("disable", *common.ENABLED, common.RECOVERY, common.AGENT, check=False)
    for name in common.UNITS | {common.RECOVERY}:
        archive(UNIT.parent / name)
        archive(UNIT.parent / (name + ".d"))
    for path in (common.HOME, common.POLICY, common.MIGRATION, common.UPDATER, common.BASELINE, common.LIB):
        archive(path)
    setup_server()
    configuration = LAB_CONFIG.read_bytes()
    halt()
    systemctl("stop", "nlup-lab-observer.service")
    systemctl("disable", *ACTIVE_UNITS, "nlup-lab-observer.service", check=False)
    directory(CONFIG.parent, "nodelite-agent", "nodelite-agent", 0o700)
    atomic_bytes(CONFIG, configuration, 0o600)
    import pwd
    user = pwd.getpwnam("nodelite-agent")
    os.chown(CONFIG, user.pw_uid, user.pw_gid)
    atomic_bytes(common.LEGACY, Path("/home/lab/real-binaries/agent-old").read_bytes(), 0o755)
    atomic_bytes(UNIT, f'''[Unit]
Description=Default manual Agent before isolated signed migration
After=network-online.target
[Service]
User=nodelite-agent
Group=nodelite-agent
ExecStart={common.LEGACY} --config {CONFIG}
WorkingDirectory={CONFIG.parent}
Restart=always
RestartSec=1
KillMode=control-group
CapabilityBoundingSet=
AmbientCapabilities=
NoNewPrivileges=yes
Environment=NODELITE_AGENT_TRAFFIC_CONTROL=0
[Install]
WantedBy=multi-user.target
'''.encode(), 0o644)
    if PUBLIC.exists() and digest((PUBLIC / "base/current/metadata/1.root.json").read_bytes()) \
            != digest((INPUT / "public/base/current/metadata/1.root.json").read_bytes()):
        archive(PUBLIC)
    if not PUBLIC.exists():
        shutil.copytree(INPUT / "public", PUBLIC)
        for path in (PUBLIC, *PUBLIC.rglob("*")):
            if not path.is_symlink():
                path.chmod(0o755 if path.is_dir() else 0o444)
    select("base")
    unit("enrollment-mirror.service", f'''[Service]
User=lab
ExecStart=/usr/bin/python3 /home/lab/agent-update-isolation/enrollment_mirror.py
NoNewPrivileges=yes
CapabilityBoundingSet=
[Install]
WantedBy=multi-user.target
''')
    systemctl("daemon-reload")
    systemctl("reset-failed", common.AGENT, "nlup-lab-enrollment.service", check=False)
    systemctl("enable", "--now", common.AGENT, "nlup-lab-enrollment-mirror.service")
    wait_for(lambda: online("3.0.4"))
    return {"legacy_ready": True, "pid": common.properties()["MainPID"]}


def select(name):
    if name not in {"base", "refresh", "rotated", "rotated-recovery", "revoked", "root-gap"}:
        raise ValueError("unknown fixed mirror")
    (PUBLIC / ".selected").symlink_to(name + "/current")
    os.replace(PUBLIC / ".selected", PUBLIC / "selected")
    (PUBLIC / "fault").unlink(missing_ok=True)


def rotate():
    before = CONFIG.read_bytes()
    assert api("/api/nodes/cold-node/refresh-token", {})["ok"]
    latest = CONFIG.read_bytes()
    assert latest != before
    return latest


def normal():
    common = production()
    latest = rotate()
    migrate("--apply")
    wait_for(lambda: online("rust-old"))
    assert json.loads(JOURNAL.read_text())["phase"] == "committed"
    assert CONFIG.read_bytes() == latest
    original_pid = common.properties()["MainPID"]
    assert migrate("--apply", check=False).returncode != 0
    assert common.properties()["MainPID"] == original_pid
    return {"scenario": "default_304_to_signed_migration", "passed": True, "latest_credentials_preserved": True,
            "repeated_migration_rejected": True}


def main():
    guard()
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("setup", "normal", "plan", "diagnose", "halt", "case", "arm", "after"))
    parser.add_argument("argument", nargs="?")
    args = parser.parse_args()
    if args.action == "setup":
        result = setup()
    elif args.action == "normal":
        result = normal()
    elif args.action in {"case", "arm", "after"}:
        if args.argument.startswith("security_"):
            import security_guest
            result = getattr(security_guest, args.action)(args.argument)
        else:
            import enrollment_checks
            result = getattr(enrollment_checks, args.action)(args.argument)
    elif args.action == "plan":
        common = production()
        pid, config = common.properties()["MainPID"], CONFIG.read_bytes()
        migrate()
        assert common.properties()["MainPID"] == pid and CONFIG.read_bytes() == config and not JOURNAL.exists()
        result = {"scenario": "dry_run", "passed": True, "service_unchanged": True}
    elif args.action == "halt":
        common = production()
        systemctl("stop", *common.UNITS, common.RECOVERY, "nlup-lab-enrollment.service", "nlup-lab-enrollment-mirror.service", check=False)
        systemctl("disable", *common.ENABLED, common.RECOVERY, common.AGENT, "nlup-lab-enrollment-mirror.service", check=False)
        stop_all()
        result = {"stopped": True}
    else:
        result = {"migration": json.loads(JOURNAL.read_text()) if JOURNAL.exists() else None,
                  "service": production().properties(),
                  "units": run("journalctl", "-u", "nodelite-updater-migration-recover.service", "--no-pager", "-n", "20").stdout}
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
