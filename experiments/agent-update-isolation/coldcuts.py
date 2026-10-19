#!/usr/bin/env python3
"""Compiler-side driver: only the marked QEMU VM loses power, never the host."""

import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess

from releases import RELEASES, TARGET
from vm import WORK, boot, command, ssh_args, stop

HERE = WORK.parents[1] / "experiments/agent-update-isolation"
EVIDENCE = WORK / "official-cold-evidence.json"


def guest(*args):
    try:
        result = command(ssh_args(WORK / "vm-v1") + [" ".join(shlex.quote(str(arg)) for arg in (
            "sudo", "env", "PYTHONPATH=/usr/local/lib/nlup-lab/python-libs", "python3",
            "/home/lab/agent-update-isolation/cold_guest.py", *args))],
            capture_output=True, text=True, timeout=100)
    except subprocess.CalledProcessError as error:
        raise RuntimeError("guest action failed: " + error.stderr.strip().splitlines()[-1]) from None
    return json.loads(result.stdout.strip().splitlines()[-1])


def main():
    if Path.cwd().resolve() != WORK.parents[1].resolve() or os.geteuid() == 0:
        raise RuntimeError("run as the unprivileged compiler from ~/NodeLite only")
    if not (WORK / "official-real-evidence.json").exists():
        raise RuntimeError("run the official real Agent suite first")
    EVIDENCE.unlink(missing_ok=True)
    command(["python3", HERE / "vm.py", "push", HERE, "/home/lab/"])
    binaries = WORK / "real-binaries"
    env = os.environ.copy()
    env["NODELITE_BUILD_VERSION"] = "lab-next"
    command(["cargo", "build", "-p", "nodelite-agent", "--release", "--locked"], cwd=WORK.parents[1], env=env)
    for name, source in (("agent-old", RELEASES / "3.0.4" / f"nodelite-agent-{TARGET}"),
                         ("agent-new", WORK.parents[1] / "target/release/nodelite-agent")):
        destination = binaries / name
        destination.chmod(0o700)
        shutil.copyfile(source, destination)
    for version in ("3.0.3", "3.0.4"):
        destination = binaries / f"server-{version}"
        if destination.exists():
            destination.chmod(0o700)
        shutil.copyfile(RELEASES / version / f"nodelite-server-{TARGET}", destination)
        destination.chmod(0o700)
    command(["python3", HERE / "vm.py", "push", binaries, "/home/lab/"])
    libs = WORK / "python-env/lib/python3.10/site-packages"
    command(["python3", HERE / "vm.py", "push", str(libs) + "/", "/home/lab/python-libs/"])
    remote = "sudo cp -r /home/lab/python-libs /usr/local/lib/nlup-lab/"
    command(ssh_args(WORK / "vm-v1") + [remote])
    results = []

    def record(result):
        results.append(result)
        EVIDENCE.write_text(json.dumps({"complete": False, "scenarios": results}, indent=2) + "\n")
        print(json.dumps(result), flush=True)

    try:
        guest("setup", "3.0.3")
        record(guest("restart", "upgrade"))
        guest("setup")
        record(guest("restart"))
        for point in ("before_delivery", "lost_ack", "prepared", "stopping", "trial_journal",
                      "active_pointer", "trial_rotated"):
            guest("setup")
            before = guest("arm", point)
            stop(WORK / "vm-v1")
            boot(WORK / "vm-v1")
            result = {"before": before, "after": guest("after")}
            record(result)
    finally:
        cleanup = guest("halt")
    if len(results) != 9:
        raise RuntimeError("incomplete cold-boot matrix")
    evidence = {"complete": True, "scenarios": results, "inventory": guest("inventory"),
                "services_stopped": cleanup["stopped"],
                "versions": {"old_agent": "3.0.4", "candidate": "lab-next", "server": "3.0.4"}}
    EVIDENCE.write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
