#!/usr/bin/env python3
"""Compiler-side TUF cache power cuts and guest-only wall-clock changes."""

import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time

from releases import RELEASES, TARGET
from resilience_guest import POINTS
from tuf_repo import Repository
from vm import WORK, boot, command, ssh_args, stop

HERE = WORK.parents[1] / "experiments/agent-update-isolation"
OUTPUT = WORK / "cache-powercut-evidence.json"


def guest(*args, timeout=100):
    result = subprocess.run([str(arg) for arg in ssh_args(WORK / "vm-v1")] + [
        " ".join(shlex.quote(str(arg)) for arg in ("sudo", "env",
        "PYTHONPATH=/usr/local/lib/nlup-lab/python-libs", "python3",
        "/home/lab/agent-update-isolation/resilience_guest.py", *args))],
        capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError("guest experiment failed: " + result.stderr[-4000:])
    if not result.stdout.strip():
        raise RuntimeError("guest controller returned no evidence")
    return json.loads(result.stdout.strip().splitlines()[-1])


def prepare():
    payloads = [(RELEASES / version / f"nodelite-agent-{TARGET}").read_bytes()
                for version in ("3.0.3", "3.0.4")]
    with tempfile.TemporaryDirectory(prefix="resilience-repository-", dir=WORK) as temporary:
        directory = Path(temporary)
        repo = Repository(directory / "publisher")
        public = directory / "resilience-repository"
        public.mkdir()
        for name, sequence, version, payload in (("one", 1, 1, payloads[0]), ("two", 2, 2, payloads[1]),
                                                ("downgrade", 1, 3, payloads[0]), ("latest", 2, 4, payloads[1])):
            repo.publish(sequence, payload, version=version)
            shutil.copytree(repo.directory, public / name)
        (public / "bootstrap.json").write_bytes(repo.bootstrap)
        (public / "payloads.json").write_text(json.dumps([hashlib.sha256(p).hexdigest() for p in payloads]))
        command(["python3", HERE / "vm.py", "push", public, "/home/lab/"])
    return [hashlib.sha256(payload).hexdigest() for payload in payloads]


def main():
    if Path.cwd().resolve() != WORK.parents[1].resolve() or os.geteuid() == 0:
        raise RuntimeError("run as compiler from ~/NodeLite only")
    OUTPUT.unlink(missing_ok=True)
    command(["python3", HERE / "vm.py", "push", HERE, "/home/lab/"])
    hashes = prepare()
    guest("setup")
    results = []
    try:
        for point in POINTS:
            armed = guest("arm", point)
            stop(WORK / "vm-v1")
            boot(WORK / "vm-v1")
            result = {"armed": armed, **guest("after", point)}
            assert result["boot_id"] != armed["boot_id"], "guest did not cold boot"
            results.append(result)
            print(json.dumps(result), flush=True)
            OUTPUT.write_text(json.dumps({"complete": False, "scenarios": results}, indent=2) + "\n")
        clock = guest("clock")
        assert abs(clock["restored_epoch"] - time.time()) < 10, "guest clock was not restored"
        print(json.dumps(clock), flush=True)
    finally:
        cleanup = guest("halt")
    assert len(results) == 5 and len(clock["clock_cases"]) == 2
    OUTPUT.write_text(json.dumps({"complete": True, "powercuts": results, "clock": clock,
        "cleanup": cleanup, "official_sha256": hashes,
        "source_sha256": {name: hashlib.sha256((HERE / name).read_bytes()).hexdigest()
                          for name in ("cache_store.py", "durable_client.py", "resilience_guest.py", "resilience_vm.py")}}, indent=2) + "\n")


if __name__ == "__main__":
    main()
