#!/usr/bin/env python3
"""Install fixtures only inside the explicitly marked disposable QEMU/KVM guest."""

import grp
import json
import os
from pathlib import Path
import pwd
import secrets
import shutil
import subprocess

from lab_io import CODE, PRIVATE, ROOT, atomic_bytes, atomic_json, digest, pointer

HERE = Path(__file__).resolve().parent
LIB = Path("/usr/local/lib/nlup-lab")
ETC = Path("/etc/nlup-lab")


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True, timeout=20)


def guard():
    marker = Path("/etc/nlup-lab-guest")
    if os.geteuid() != 0 or not marker.exists() \
            or marker.read_text().strip() != "isolated-prototype-v1" \
            or run("systemd-detect-virt", "--vm").stdout.strip() not in {"kvm", "qemu"}:
        raise RuntimeError("refusing installation outside the marked disposable VM")


def directory(path, owner, group, mode):
    path.mkdir(parents=True, exist_ok=True)
    os.chown(path, pwd.getpwnam(owner).pw_uid, grp.getgrnam(group).gr_gid)
    path.chmod(mode)


def unit(name, content):
    path = Path("/etc/systemd/system") / f"nlup-lab-{name}"
    path.write_text(content)
    path.chmod(0o644)


def agent_units(common):
    unit("agent.service", f"""[Unit]
Description=Disposable updater Agent fixture
After=nlup-lab-boot.service
Requires=nlup-lab-boot.service
OnFailure=nlup-lab-reconcile.service
StartLimitIntervalSec=30
StartLimitBurst=3
[Service]
User=nlup-agent
Group=nlup-read
SupplementaryGroups=nlup-agent nlup-requests
ExecStart={CODE}/active
Restart=always
RestartSec=300ms
TimeoutStopSec=2
UMask=0027
ReadWritePaths={ROOT}/agent {ROOT}/status {ROOT}/inbox
{common}
[Install]
WantedBy=multi-user.target
""")
    unit("preflight.service", f"""[Unit]
Description=Disposable candidate without credentials
[Service]
Type=oneshot
RemainAfterExit=yes
User=nlup-agent
Group=nlup-read
ExecStart={CODE}/candidate --preflight
TimeoutStartSec=3
{common}
""")


def worker_units(common):
    for name in ("request", "reconcile", "boot"):
        before = "Before=nlup-lab-agent.service\n" if name == "boot" else ""
        argument = " --boot" if name == "boot" else ""
        unit(f"{name}.service", f"""[Unit]
Description=Disposable unprivileged updater {name}
{before}StartLimitIntervalSec=30
StartLimitBurst=50
[Service]
Type=oneshot
User=nlup-updater
Group=nlup-read
SupplementaryGroups=nlup-updater nlup-requests
Environment=PYTHONDONTWRITEBYTECODE=1
ExecStart=/usr/bin/python3 {LIB}/updater.py{argument}
ReadWritePaths={CODE} {PRIVATE} {ROOT}/bridge {ROOT}/inbox
TimeoutStartSec=15
{common}
""")
    unit("request.path", f"""[Path]
PathExists={ROOT}/inbox/request.json
[Install]
WantedBy=multi-user.target
""")
    unit("reconcile.timer", """[Timer]
OnBootSec=2
OnUnitInactiveSec=1
AccuracySec=100ms
Unit=nlup-lab-reconcile.service
[Install]
WantedBy=timers.target
""")


def bridge_units():
    for name, target in (("restart", "agent"), ("preflight", "preflight")):
        # These root actions never execute or source code from the updater's directories.
        unit(f"{name}-bridge.service", f"""[Unit]
StartLimitIntervalSec=30
StartLimitBurst=10
[Service]
Type=oneshot
ExecStart=/usr/bin/rm -f -- {ROOT}/bridge/{name}
ExecStart=-/usr/bin/systemctl reset-failed nlup-lab-{target}.service
ExecStart=/usr/bin/systemctl --no-block restart nlup-lab-{target}.service
NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths={ROOT}/bridge
ProtectHome=yes
PrivateTmp=yes
""")
        unit(f"{name}-bridge.path", f"""[Path]
PathExists={ROOT}/bridge/{name}
[Install]
WantedBy=multi-user.target
""")


def baseline():
    executable = (HERE / "fixture/target/release/nodelite-update-lab-fixture").read_bytes()
    payload = executable + b'\nNLUP_FIXTURE={"sequence":1,"mode":"healthy"}'
    identity = digest(payload)
    destination = CODE / "objects" / identity
    directory(destination, "nlup-updater", "nlup-read", 0o750)
    atomic_bytes(destination / "agent", payload, 0o550)
    os.chown(destination / "agent", pwd.getpwnam("nlup-updater").pw_uid, grp.getgrnam("nlup-read").gr_gid)
    pointer("active", identity)
    os.lchown(CODE / "active", pwd.getpwnam("nlup-updater").pw_uid, grp.getgrnam("nlup-read").gr_gid)
    state = {"schema": 1, "generation": 1, "phase": "stable", "current": identity,
             "sequence": 1, "metadata_high": 1, "failed": [], "jobs": [], "attempts": []}
    atomic_json(PRIVATE / "baseline.json", state)
    atomic_json(PRIVATE / "journal.json", state)
    for path in PRIVATE.iterdir():
        os.chown(path, pwd.getpwnam("nlup-updater").pw_uid, grp.getgrnam("nlup-updater").gr_gid)


def main():
    guard()
    for group in ("nlup-agent", "nlup-updater", "nlup-read", "nlup-requests"):
        if subprocess.run(["getent", "group", group], capture_output=True).returncode:
            run("groupadd", "--system", group)
    for user in ("nlup-agent", "nlup-updater"):
        if subprocess.run(["id", user], capture_output=True).returncode:
            run("useradd", "--system", "--no-create-home", "--shell", "/usr/sbin/nologin",
                "--gid", user, "--groups", "nlup-read,nlup-requests", user)
    directory(LIB, "root", "root", 0o755)
    directory(ETC, "root", "root", 0o755)
    directory(ROOT, "root", "root", 0o755)
    for path, owner, group, mode in (
        (CODE, "nlup-updater", "nlup-read", 0o750),
        (CODE / "objects", "nlup-updater", "nlup-read", 0o750),
        (PRIVATE, "nlup-updater", "nlup-updater", 0o700),
        (ROOT / "agent", "nlup-agent", "nlup-agent", 0o700),
        (ROOT / "status", "nlup-agent", "nlup-read", 0o750),
        (ROOT / "inbox", "nlup-agent", "nlup-requests", 0o2770),
        (ROOT / "bridge", "root", "nlup-updater", 0o770),
    ):
        directory(path, owner, group, mode)
    for name in ("updater.py", "lab_io.py"):
        shutil.copyfile(HERE / name, LIB / name)
        (LIB / name).chmod(0o555)
    atomic_bytes(ROOT / "agent/config", secrets.token_bytes(48))
    os.chown(ROOT / "agent/config", pwd.getpwnam("nlup-agent").pw_uid, grp.getgrnam("nlup-agent").gr_gid)
    key = Path("/root/nlup-lab-signing.pem")
    if not key.exists():
        run("openssl", "genpkey", "-algorithm", "ED25519", "-out", str(key))
        key.chmod(0o600)
    run("openssl", "pkey", "-in", str(key), "-pubout", "-out", str(ETC / "release.pub"))
    atomic_json(ETC / "policy.json", {"minimum_sequence": 1})
    (ETC / "policy.json").chmod(0o644)
    baseline()
    common = """NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
CapabilityBoundingSet=
AmbientCapabilities=
RestrictSUIDSGID=yes
RestrictNamespaces=yes
"""
    agent_units(common)
    worker_units(common)
    bridge_units()
    run("systemctl", "daemon-reload")
    run("systemctl", "enable", "--now", "nlup-lab-agent.service", "nlup-lab-request.path",
        "nlup-lab-reconcile.timer", "nlup-lab-restart-bridge.path", "nlup-lab-preflight-bridge.path")
    print(json.dumps({"installed": True, "scope": "disposable VM only"}))


if __name__ == "__main__":
    main()
