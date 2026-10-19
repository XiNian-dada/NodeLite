#!/usr/bin/env python3
"""Compiler driver for five bounded filesystem exhaustion experiments, no host mounts."""

import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess

from releases import RELEASES, TARGET
from vm import WORK, command, ssh_args

HERE = WORK.parents[1] / "experiments/agent-update-isolation"


def prepare():
    binaries = WORK / "real-binaries"
    env = os.environ.copy()
    env["NODELITE_BUILD_VERSION"] = "lab-next"
    command(["cargo", "build", "-p", "nodelite-agent", "--release", "--locked"], env=env)
    for name, source in (("agent-old", RELEASES / "3.0.4" / f"nodelite-agent-{TARGET}"),
                         ("agent-new", WORK.parents[1] / "target/release/nodelite-agent")):
        destination = binaries / name
        if destination.exists():
            destination.chmod(0o700)
        shutil.copyfile(source, destination)
        destination.chmod(0o700)
    command(["python3", HERE / "vm.py", "push", binaries, "/home/lab/"])
    command(["python3", HERE / "vm.py", "push", HERE, "/home/lab/"])


def main():
    if Path.cwd().resolve() != WORK.parents[1].resolve() or os.geteuid() == 0:
        raise RuntimeError("run as compiler from ~/NodeLite only")
    output = WORK / "storage-resilience-evidence.json"
    output.unlink(missing_ok=True)
    prepare()
    results = []
    for case in ("cache_disk", "cache_inode", "cache_disk_target", "token_disk", "token_inode"):
        arguments = ("sudo", "env", "PYTHONPATH=/usr/local/lib/nlup-lab/python-libs", "python3",
                     "/home/lab/agent-update-isolation/resilience_storage.py", case)
        result = subprocess.run([str(arg) for arg in ssh_args(WORK / "vm-v1")] + [
            " ".join(shlex.quote(arg) for arg in arguments)], capture_output=True, text=True, timeout=120)
        if result.returncode:
            raise RuntimeError("storage experiment failed: " + result.stderr[-3000:])
        evidence = json.loads(result.stdout.strip().splitlines()[-1])
        assert evidence["scenario"] == case and evidence["passed"]
        results.append(evidence)
        output.write_text(json.dumps({"complete": False, "scenarios": results}, indent=2) + "\n")
        print(json.dumps(evidence), flush=True)
    assert len(results) == 5
    output.write_text(json.dumps({"complete": True, "scenarios": results,
        "source_sha256": {name: hashlib.sha256((HERE / name).read_bytes()).hexdigest() for name in (
            "resilience_volume.py", "resilience_storage.py", "storage_vm.py", "cache_store.py", "durable_client.py")},
        "official_agent_sha256": hashlib.sha256((WORK / "real-binaries/agent-old").read_bytes()).hexdigest()}, indent=2) + "\n")


if __name__ == "__main__":
    main()
