#!/usr/bin/env python3
"""Trusted controller and unprivileged downloader for the disposable resilience VM."""

import argparse
import json
import os
from pathlib import Path
import pwd
import secrets
import shutil
import signal
import time

from cache_store import Store
from durable_client import DurableClient
import lab_io
from setup_guest import LIB, directory, guard, run, unit

ROOT = Path("/var/lib/nlup-resilience")
REPO = ROOT / "repository"
RUNTIME = Path("/usr/local/lib/nlup-resilience")
HERE = Path(__file__).resolve().parent
POINTS = ("metadata_staged", "state_renamed", "metadata_committed", "object_durable", "artifact_committed")
MARKER = Path("/run/nlup-resilience/point.json")
BASE = "http://127.0.0.1:18092/"


def bootstrap():
    return (REPO / "bootstrap.json").read_bytes()


def summary(cache):
    store = Store(cache, bootstrap())
    state = store.load()
    return {"sequence": state["sequence"], "artifact": state["artifact"], "time_high": state["time_high"]}


def worker(cache, revision, point="none"):
    if os.geteuid() != pwd.getpwnam("nlup-updater").pw_uid:
        raise RuntimeError("downloader must use the dedicated non-sudo updater UID")
    capabilities = next(line.split()[1] for line in Path("/proc/self/status").read_text().splitlines()
                        if line.startswith("CapEff:"))
    assert int(capabilities, 16) == 0
    def checkpoint(name):
        if name == point:
            # tmpfs notification must not flush the disk whose write ordering is under test.
            MARKER.write_text(json.dumps({"point": name}))
            signal.pause()

    if point == "state_renamed":
        original = lab_io.sync_dir

        def sync(directory):
            if directory == cache:
                checkpoint("state_renamed")
            original(directory)
        lab_io.sync_dir = sync
    try:
        DurableClient(cache, BASE + revision, bootstrap(), checkpoint).download()
        result = {"success": True}
    except Exception as error:
        result = {"success": False, "error": type(error).__name__, "errno": getattr(error, "errno", None)}
    return {**result, "cache": summary(cache), "uid": os.geteuid(), "effective_capabilities": capabilities}


def invoke(cache, revision):
    result = run("runuser", "-u", "nlup-updater", "--", "env", f"PYTHONPATH={LIB}/python-libs",
                 "python3", str(RUNTIME / "resilience_guest.py"), "worker", str(cache), revision)
    return json.loads(result.stdout.strip().splitlines()[-1])


def setup():
    from cold_support import stop_all
    guard()
    stop_all()
    directory(ROOT, "root", "root", 0o755)
    directory(RUNTIME, "root", "root", 0o755)
    for name in ("cache_store.py", "durable_client.py", "lab_io.py", "network_deadline.py",
                 "tuf_client.py", "tuf_repo.py", "setup_guest.py", "resilience_guest.py"):
        shutil.copyfile(HERE / name, RUNTIME / name)
        (RUNTIME / name).chmod(0o555)
    shutil.copytree("/home/lab/resilience-repository", REPO, dirs_exist_ok=True)
    for path in (REPO, *REPO.rglob("*")):
        path.chmod(0o755 if path.is_dir() else 0o444)
    unit("resilience-mirror.service", f"""[Unit]
Description=Disposable loopback signed repository
After=network.target
[Service]
User=nlup-updater
ExecStart=/usr/bin/python3 -m http.server 18092 --bind 127.0.0.1 --directory {REPO}
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
CapabilityBoundingSet=
Restart=on-failure
[Install]
WantedBy=multi-user.target
""")
    run("systemctl", "daemon-reload")
    run("systemctl", "enable", "--now", "nlup-lab-resilience-mirror.service")
    # Freeze durable test infrastructure before cutting power to cache transactions.
    os.sync()
    return {"setup": True}


def provision(name):
    if name not in POINTS + ("clock",):
        raise ValueError("unknown experiment cache")
    cache = ROOT / name
    if cache.exists():
        cache.rename(ROOT / (name + ".archived." + secrets.token_hex(8)))
        lab_io.sync_dir(ROOT)
    directory(cache, "nlup-updater", "nlup-updater", 0o700)
    Store(cache, bootstrap()).provision(time.time())
    identity = pwd.getpwnam("nlup-updater")
    for path in cache.rglob("*"):
        os.chown(path, identity.pw_uid, identity.pw_gid)
    result = invoke(cache, "one")
    assert result["success"] and result["cache"]["sequence"] == 1
    return result


def arm(point):
    provision(point)
    directory(MARKER.parent, "nlup-updater", "nlup-updater", 0o700)
    MARKER.unlink(missing_ok=True)
    unit("resilience-worker.service", f"""[Service]
User=nlup-updater
Environment=PYTHONPATH={LIB}/python-libs PYTHONDONTWRITEBYTECODE=1
ExecStart=/usr/bin/python3 {RUNTIME}/resilience_guest.py worker {ROOT / point} two {point}
NoNewPrivileges=yes
CapabilityBoundingSet=
KillMode=control-group
""")
    run("systemctl", "daemon-reload")
    os.sync()
    run("systemctl", "restart", "nlup-lab-resilience-worker.service")
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if MARKER.exists():
            assert json.loads(MARKER.read_text())["point"] == point
            return {"armed": point, "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip()}
        time.sleep(0.05)
    raise RuntimeError("worker did not reach the requested power-cut boundary")


def after(point):
    cache = ROOT / point
    before = summary(cache)
    assert before["sequence"] in (1, 2)
    assert before["artifact"]["sequence"] in (1, 2)
    rejection = None
    if before["sequence"] == 2:
        downgrade = invoke(cache, "downgrade")
        assert not downgrade["success"] and downgrade["error"] == "LabPolicyError"
        rejection = downgrade["error"]
    result = invoke(cache, "latest")
    assert result["success"] and result["cache"]["sequence"] == 2
    assert result["cache"]["artifact"]["sequence"] == 2
    assert result["cache"]["artifact"]["sha256"] == json.loads((REPO / "payloads.json").read_text())[1]
    return {"point": point, "passed": True, "cold_cache": before,
            "downgrade_rejected_by": rejection, "retry": result,
            "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip()}


def clock():
    provision("clock")
    cache = ROOT / "clock"
    before = summary(cache)
    run("timedatectl", "set-ntp", "false")
    reference, monotonic = time.time(), time.monotonic()
    results = []
    try:
        for offset, expected in ((-7200, "ClockRollback"), (172800, "ExpiredMetadataError")):
            run("date", "-u", "-s", f"@{reference + offset}")
            result = invoke(cache, "two")
            assert not result["success"] and result["error"] == expected, result
            assert result["cache"]["artifact"] == before["artifact"]
            assert result["cache"]["time_high"] == before["time_high"]
            results.append({"offset_seconds": offset, "rejected_by": result["error"], "passed": True})
    finally:
        run("date", "-u", "-s", f"@{reference + time.monotonic() - monotonic}")
    retry = invoke(cache, "latest")
    assert retry["success"] and retry["cache"]["artifact"]["sequence"] == 2
    return {"clock_cases": results, "restored_epoch": time.time(), "retry": retry}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("setup", "arm", "after", "clock", "worker", "halt"))
    parser.add_argument("arguments", nargs="*")
    args = parser.parse_args()
    if args.action == "worker":
        cache, revision, *point = args.arguments
        result = worker(Path(cache), revision, point[0] if point else "none")
    else:
        guard()
        if args.action == "setup":
            result = setup()
        elif args.action == "arm":
            result = arm(*args.arguments)
        elif args.action == "after":
            result = after(*args.arguments)
        elif args.action == "clock":
            result = clock()
        else:
            run("systemctl", "stop", "nlup-lab-resilience-worker.service", "nlup-lab-resilience-mirror.service")
            run("systemctl", "disable", "nlup-lab-resilience-mirror.service")
            result = {"stopped": True}
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
