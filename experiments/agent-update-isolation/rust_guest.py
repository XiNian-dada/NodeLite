#!/usr/bin/env python3
"""Install opt-in Rust units only inside the already-marked disposable KVM guest."""

import argparse
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import time

from cold_support import CONFIG, OLD, api, setup as setup_server, stop_all, wait_for
from lab_io import atomic_bytes, atomic_json, digest
from real_guest import halt, own
from setup_guest import directory, guard, run, unit
from test_guest import ACTIVE_UNITS, systemctl

HOME = Path("/var/lib/nodelite-updater")
AGENT = Path("/var/lib/nodelite-agent")
POLICY = Path("/etc/nodelite-updater")
INPUT = Path("/home/lab/rust-updater-input")
PUBLIC = Path("/var/lib/nlup-rust-repository")
BINARIES = Path("/home/lab/rust-binaries")
STATE = HOME / "private/journal.json"
UNITS = ("nodelite-agent.service", "nodelite-agent-preflight.service", "nodelite-updater-reconcile.timer",
         "nodelite-updater-reconcile.service", "nodelite-updater-boot.service",
         "nodelite-updater-stop.path", "nodelite-updater-restart.path", "nodelite-updater-preflight.path",
         "nodelite-updater-stop.service", "nodelite-updater-restart.service", "nodelite-updater-preflight.service",
         "nodelite-updater-apply.service")


def updater(*args, success=True):
    result = subprocess.run(["runuser", "-u", "nodelite-updater", "-g", "nodelite-read", "-G", "nodelite-updater", "--", "/usr/local/bin/nodelite-updater", *args],
                            capture_output=True, text=True, timeout=70)
    if success and result.returncode:
        raise RuntimeError("Rust updater rejected: " + result.stderr[-1200:])
    return result


def state():
    return json.loads(STATE.read_text())


def health(version):
    status = api("/api/nodes/cold-node")
    if not status.get("snapshot") or not status.get("online"):
        return False
    process = run("systemctl", "show", "nodelite-agent.service", "--property=MainPID,InvocationID").stdout
    process = dict(line.split("=", 1) for line in process.splitlines())
    evidence = json.loads(Path("/run/nodelite-agent/update-health.json").read_text())
    return status["identity"]["agent_version"] == version and str(evidence["pid"]) == process["MainPID"] \
        and evidence["invocation"] == process["InvocationID"]


def halt_all():
    systemctl("stop", *UNITS, check=False)
    systemctl("disable", *(name for name in UNITS if name.endswith((".path", ".timer"))), check=False)
    systemctl("disable", "nodelite-agent.service", check=False)
    systemctl("stop", "nlup-lab-rust-mirror.service", check=False)
    systemctl("disable", "nlup-lab-rust-mirror.service", check=False)
    stop_all()


def enroll():
    guard()
    halt_all()
    for name in ("apply", "reconcile", "boot"):
        (Path("/etc/systemd/system") / f"nodelite-updater-{name}.service.d/lab-fault.conf").unlink(missing_ok=True)
    setup_server()
    configuration = CONFIG.read_bytes()
    halt()
    systemctl("stop", "nlup-lab-observer.service")
    systemctl("disable", *ACTIVE_UNITS, "nlup-lab-observer.service", check=False)
    for group in ("nodelite-agent", "nodelite-updater", "nodelite-read"):
        if subprocess.run(["getent", "group", group], capture_output=True).returncode:
            run("groupadd", "--system", group)
    for user in ("nodelite-agent", "nodelite-updater"):
        if subprocess.run(["id", user], capture_output=True).returncode:
            run("useradd", "--system", "--no-create-home", "--shell", "/usr/sbin/nologin", "--gid", user,
                "--groups", "nodelite-read", user)
    if HOME.exists():
        HOME.rename(HOME.with_name("nodelite-updater.archived." + secrets.token_hex(8)))
    for path, user, group, mode in ((HOME, "root", "root", 0o755),
            (HOME / "private", "nodelite-updater", "nodelite-updater", 0o700),
            (HOME / "code", "nodelite-updater", "nodelite-read", 0o750),
            (HOME / "code/objects", "nodelite-updater", "nodelite-read", 0o750),
            (HOME / "bridge", "root", "nodelite-updater", 0o770),
            (AGENT, "nodelite-agent", "nodelite-agent", 0o700), (POLICY, "root", "root", 0o755)):
        directory(path, user, group, mode)
    atomic_bytes(AGENT / "agent.toml", configuration)
    own(AGENT / "agent.toml", "nodelite-agent", 0o600, "nodelite-agent")
    shutil.copyfile(BINARIES / "nodelite-updater", "/usr/local/bin/nodelite-updater")
    Path("/usr/local/bin/nodelite-updater").chmod(0o555)
    shutil.copyfile(BINARIES / "agent-old", "/usr/local/lib/nodelite-updater-baseline")
    Path("/usr/local/lib/nodelite-updater-baseline").chmod(0o555)
    for path in (INPUT / "systemd").iterdir():
        shutil.copyfile(path, Path("/etc/systemd/system") / path.name)
        (Path("/etc/systemd/system") / path.name).chmod(0o644)
    anchor = digest((INPUT / "bootstrap.json").read_bytes())
    if not (PUBLIC / "anchor").exists() or (PUBLIC / "anchor").read_text() != anchor:
        if PUBLIC.exists():
            PUBLIC.rename(PUBLIC.with_name("nlup-rust-repository.archived." + secrets.token_hex(8)))
        PUBLIC.mkdir(mode=0o755)
        shutil.copytree(INPUT / "variants", PUBLIC / "variants")
        for path in (PUBLIC, *PUBLIC.rglob("*")):
            path.chmod(0o755 if path.is_dir() else 0o444)
        atomic_bytes(PUBLIC / "anchor", anchor.encode(), 0o444)
    for path in (PUBLIC / "fault", POLICY / "fault"):
        path.unlink(missing_ok=True)
    (PUBLIC / ".current").symlink_to("variants/good")
    os.replace(PUBLIC / ".current", PUBLIC / "current")
    atomic_bytes(POLICY / "root.json", (INPUT / "bootstrap.json").read_bytes(), 0o644)
    baseline = digest((BINARIES / "agent-old").read_bytes())
    text = f'''enabled = true
metadata_url = "http://127.0.0.1:18092/metadata/"
targets_url = "http://127.0.0.1:18092/targets/"
channel = "stable"
target = "x86_64-unknown-linux-musl"
baseline_sha256 = "{baseline}"
baseline_sequence = 1
minimum_sequence = 1
trial_seconds = 25
stability_seconds = 5
'''
    atomic_bytes(POLICY / "policy.toml", text.encode(), 0o644)
    unit("rust-mirror.service", f"""[Service]
User=lab
ExecStart=/usr/bin/python3 /usr/local/lib/nlup-lab/rust_mirror.py
NoNewPrivileges=yes
CapabilityBoundingSet=
[Install]
WantedBy=multi-user.target
""")
    systemctl("daemon-reload")
    shutil.copyfile(Path(__file__).with_name("rust_mirror.py"), "/usr/local/lib/nlup-lab/rust_mirror.py")
    Path("/usr/local/lib/nlup-lab/rust_mirror.py").chmod(0o555)
    updater("initialize", "--baseline", "/usr/local/lib/nodelite-updater-baseline")
    run("systemd-analyze", "verify", *(str(Path("/etc/systemd/system") / name) for name in UNITS))
    os.sync()
    systemctl("reset-failed", *UNITS, check=False)
    systemctl("enable", "--now", "nlup-lab-rust-mirror.service", "nodelite-agent.service",
              "nodelite-updater-reconcile.timer", "nodelite-updater-stop.path", "nodelite-updater-restart.path",
              "nodelite-updater-preflight.path")
    wait_for(lambda: health("rust-old"))
    return {"enrolled": True, "baseline": baseline}


def activate():
    original = (AGENT / "agent.toml").read_bytes()
    assert api("/api/nodes/cold-node/refresh-token", {})["ok"]
    latest = (AGENT / "agent.toml").read_bytes()
    assert latest != original
    updater("apply")
    wait_for(lambda: state()["phase"] == "committed", 50)
    wait_for(lambda: health("rust-new"))
    assert (AGENT / "agent.toml").read_bytes() == latest
    updater("reconcile")
    assert state()["phase"] == "committed"
    return {"scenario": "signed_download_to_activation", "passed": True, "phase": "committed",
            "latest_credentials_preserved": True, "installed": state()["installed"],
            "health": json.loads(Path("/run/nodelite-agent/update-health.json").read_text())}


def main():
    guard()
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("enroll", "activate", "halt", "diagnose", "case", "arm", "after"))
    parser.add_argument("argument", nargs="?")
    args = parser.parse_args()
    if args.action == "enroll":
        result = enroll()
    elif args.action == "activate":
        result = activate()
    elif args.action == "halt":
        halt_all()
        result = {"stopped": True}
    elif args.action in {"case", "arm", "after"}:
        if args.action == "case" and args.argument in {"storage_disk", "storage_inode", "storage_target"}:
            import rust_storage
            result = rust_storage.case(args.argument)
        else:
            import rust_checks
            result = getattr(rust_checks, args.action)(args.argument)
    else:
        result = {"state": state() if STATE.exists() else None, "updater": updater("status", success=False).stderr,
                  "journal": run("journalctl", "-u", "nodelite-updater-reconcile.service", "--no-pager", "-n", "15").stdout}
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
