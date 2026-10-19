#!/usr/bin/env python3
"""Test control plane for real Agent binaries, only inside the marked disposable VM."""

import json
import os
from pathlib import Path
import secrets
import sys
import time

from lab_io import (BRIDGE, CODE, INBOX, JOURNAL, PRIVATE, ROOT, atomic_bytes,
                    atomic_json, digest, object_path, pointer)
from setup_guest import guard
from test_guest import ACTIVE_UNITS, own, permissions, run, systemctl

BINARIES = Path("/home/lab/real-binaries")
CONFIG = ROOT / "agent/agent.toml"


def halt():
    systemctl("stop", *ACTIVE_UNITS, check=False)
    for path in (PRIVATE / "manual.json", PRIVATE / "rejection.json", PRIVATE / "checkpoint.json",
                 INBOX / "request.json", BRIDGE / "restart", BRIDGE / "preflight", BRIDGE / "stop",
                 Path("/etc/nlup-lab/fault"), ROOT / "status/heartbeat.json"):
        path.unlink(missing_ok=True)


def dropin(name, content):
    directory = Path(f"/etc/systemd/system/nlup-lab-{name}.service.d")
    directory.mkdir(mode=0o755, exist_ok=True)
    atomic_bytes(directory / "real.conf", content.encode(), 0o644)


def provision(config):
    halt()
    identities = {}
    for version in ("old", "new"):
        payload = (BINARIES / f"agent-{version}").read_bytes()
        identity = digest(payload)
        destination = object_path(identity)
        destination.parent.mkdir(mode=0o750, exist_ok=True)
        own(destination.parent, group="nlup-read", mode=0o750)
        atomic_bytes(destination, payload, 0o550)
        own(destination, group="nlup-read", mode=0o550)
        identities[version] = identity
    atomic_bytes(CONFIG, config.encode())
    own(CONFIG, "nlup-agent", group="nlup-agent")
    (ROOT / "agent").chmod(0o700)
    state = {"schema": 1, "generation": 1, "phase": "stable", "current": identities["old"],
             "sequence": 1, "metadata_high": 1, "failed": [], "jobs": [], "attempts": []}
    atomic_json(JOURNAL, state)
    own(JOURNAL)
    pointer("active", identities["old"])
    dropin("agent", f"[Service]\nExecStart=\nExecStart={CODE}/active --config {CONFIG}\n"
           "TimeoutStopSec=5\nEnvironment=RUST_LOG=off\n")
    dropin("preflight", f"[Service]\nExecStart=\nExecStart={CODE}/candidate --check-credentials --config {CONFIG}\n"
           f"SupplementaryGroups=nlup-agent\nReadOnlyPaths={ROOT}/agent\nTimeoutStartSec=5\nEnvironment=RUST_LOG=off\n")
    systemctl("daemon-reload")
    systemctl("reset-failed", *ACTIVE_UNITS, "nlup-lab-boot.service", check=False)
    systemctl("start", "nlup-lab-agent.service", "nlup-lab-request.path", "nlup-lab-reconcile.timer",
              "nlup-lab-preflight-bridge.path", "nlup-lab-restart-bridge.path", "nlup-lab-stop-bridge.path")
    return identities


def request():
    job = secrets.token_hex(16)
    directory = INBOX / job
    directory.mkdir(mode=0o750)
    own(directory, "nlup-agent", group="nlup-requests", mode=0o750)
    payload = (BINARIES / "agent-new").read_bytes()
    atomic_bytes(directory / "agent", payload, 0o640)
    manifest = {"schema": 1, "product": "nodelite-lab", "component": "agent", "channel": "stable",
                "target": "x86_64-unknown-linux-gnu", "release_sequence": 2,
                "metadata_sequence": 2, "sha256": digest(payload), "size": len(payload),
                "not_after": int(time.time()) + 3600}
    atomic_json(directory / "manifest.json", manifest)
    run("openssl", "pkeyutl", "-sign", "-inkey", "/root/nlup-lab-signing.pem", "-rawin", "-in",
        str(directory / "manifest.json"), "-out", str(directory / "manifest.sig"))
    for path in directory.iterdir():
        own(path, "nlup-agent", group="nlup-requests", mode=0o640)
    atomic_json(INBOX / "request.json", {"job": job})
    own(INBOX / "request.json", "nlup-agent", group="nlup-requests", mode=0o640)
    return {"job": job}


def probe():
    result = systemctl("show", "nlup-lab-agent.service", "--property=MainPID,ActiveState,InvocationID")
    values = dict(line.split("=", 1) for line in result.stdout.splitlines())
    pid = int(values.get("MainPID", "0"))
    values["pid"] = pid
    if pid:
        try:
            values["digest"] = digest(Path(f"/proc/{pid}/exe").read_bytes())
        except FileNotFoundError:
            values["pid"] = 0
    return values


def main():
    guard()
    action = sys.argv[1]
    result = {}
    if action == "provision":
        result = provision(sys.stdin.read())
    elif action == "request":
        result = request()
    elif action == "probe":
        result = probe()
    elif action == "health":
        atomic_json(ROOT / "status/heartbeat.json", json.load(sys.stdin))
        own(ROOT / "status/heartbeat.json", "nlup-agent", group="nlup-read", mode=0o640)
    elif action == "state":
        result = json.loads(JOURNAL.read_text())
    elif action == "halt":
        halt()
    elif action == "restore-fixture":
        halt()
        for name in ("agent", "preflight"):
            Path(f"/etc/systemd/system/nlup-lab-{name}.service.d/real.conf").unlink(missing_ok=True)
        from setup_guest import main as setup
        setup()
        return
    elif action == "permissions":
        permissions()
        denied = run("runuser", "-u", "nlup-updater", "--", "test", "-r", str(CONFIG), check=False)
        assert denied.returncode != 0, "updater can directly read actual credential file"
        assert CONFIG.stat().st_mode & 0o777 == 0o600
    elif action == "config":
        atomic_bytes(CONFIG, sys.stdin.read().encode())
        own(CONFIG, "nlup-agent", group="nlup-agent")
    elif action == "writable":
        (ROOT / "agent").chmod(0o700 if sys.argv[2] == "yes" else 0o500)
    elif action == "signal":
        if sys.argv[2] not in {"SIGSTOP", "SIGKILL", "SIGTERM"}:
            raise ValueError("unsupported lab signal")
        systemctl("kill", "--kill-who=all", "--signal=" + sys.argv[2], "nlup-lab-agent.service")
    elif action == "restart":
        systemctl("restart", "nlup-lab-agent.service")
    elif action == "fault":
        if sys.argv[2] not in {"stopping", "trial_journal", "active_pointer"}:
            raise ValueError("unsupported checkpoint")
        atomic_bytes(Path("/etc/nlup-lab/fault"), sys.argv[2].encode(), 0o644)
    elif action == "clear-fault":
        Path("/etc/nlup-lab/fault").unlink(missing_ok=True)
    else:
        raise ValueError("unsupported lab action")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
