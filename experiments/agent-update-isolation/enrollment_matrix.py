#!/usr/bin/env python3
"""Compiler-only operator ceremonies and first migration in a distinct disposable overlay."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shlex
import subprocess

from enrollment_publisher import build
from vm import WORK, boot, command, ssh_args, stop

PROJECT = WORK.parents[1]
VM = WORK / "vm-enrollment-v1"
HERE = PROJECT / "experiments/agent-update-isolation"


def push(source, destination):
    transport = " ".join(shlex.quote(str(arg)) for arg in ssh_args(VM)[:-1])
    command(["rsync", "-rl", "--exclude=__pycache__", "--exclude=*.pyc", "-e", transport, source, "lab@127.0.0.1:" + destination])


def guest(*args):
    result = subprocess.run([str(arg) for arg in ssh_args(VM)] + [
        "sudo env PYTHONPATH=/usr/local/lib/nlup-lab/python-libs python3 /home/lab/agent-update-isolation/enrollment_guest.py "
        + " ".join(shlex.quote(str(arg)) for arg in args)], capture_output=True, text=True, timeout=180)
    if result.returncode:
        raise RuntimeError(result.stderr[-5000:])
    return json.loads(result.stdout.strip().splitlines()[-1])


def source_hashes():
    paths = list((PROJECT / "tools").rglob("*.py"))
    paths += list(HERE.glob("enrollment*.py")) + list((PROJECT / "nodelite-updater/src").glob("*.rs"))
    paths += list((PROJECT / "nodelite-updater/systemd").glob("*"))
    paths += [PROJECT / "Cargo.toml", PROJECT / "Cargo.lock", PROJECT / "tools/nodelite_release/requirements.txt"]
    for component in ("nodelite-agent", "nodelite-proto"):
        paths += list((PROJECT / component / "src").rglob("*.rs"))
        paths += [PROJECT / component / "Cargo.toml"]
    return {str(path.relative_to(PROJECT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}


def prepare():
    source = source_hashes()
    prior = json.loads((WORK / "rust-binaries/build.json").read_text())
    reused = {name: value for name, value in prior["source_sha256"].items()
              if name.startswith(("nodelite-agent/", "nodelite-proto/")) or name in {"Cargo.toml", "Cargo.lock"}}
    assert reused and all(source.get(name) == value for name, value in reused.items()), "Agent source changed; rebuild real payloads"
    command(["cargo", "build", "-p", "nodelite-updater", "--release", "--locked"], env={**os.environ, "CARGO_BUILD_JOBS": "4"})
    output = WORK / "enrollment-input"
    if output.exists():
        output.rename(output.with_name("enrollment-input.archived." + secrets.token_hex(8)))
    build(output)
    push(output, "/home/lab/")
    push(PROJECT / "tools", "/home/lab/enrollment-tools/")
    push(HERE, "/home/lab/")
    command(ssh_args(VM) + ["sudo mkdir -p /root/nlup-enrollment-operator && sudo cp -r /home/lab/enrollment-tools/tools /root/nlup-enrollment-operator/ && sudo chmod -R go-w /root/nlup-enrollment-operator"])
    assert source_hashes() == source
    (WORK / "enrollment-build.json").write_text(json.dumps({"source_sha256": source,
        "normal_updater_sha256": hashlib.sha256((PROJECT / "target/release/nodelite-updater").read_bytes()).hexdigest()}, indent=2) + "\n")


def main():
    if Path.cwd().resolve() != PROJECT.resolve() or os.geteuid() == 0:
        raise RuntimeError("run only as compiler from ~/NodeLite")
    if (WORK / "vm-v1/qemu.pid").exists():
        raise RuntimeError("the earlier validation VM must remain stopped")
    args = argparse.ArgumentParser()
    args.add_argument("--prepare", action="store_true")
    options = args.parse_args()
    if options.prepare:
        prepare()
    results = []
    evidence = WORK / "enrollment-evidence.json"
    evidence.write_text(json.dumps({"complete": False, "scenarios": results}) + "\n")

    def record(result):
        results.append(result)
        evidence.write_text(json.dumps({"complete": False, "scenarios": results}, indent=2) + "\n")
        print(json.dumps(result), flush=True)

    try:
        guest("setup")
        record(guest("plan"))
        record(guest("normal"))
        for name in ("timestamp_refresh", "root_rotation", "revoked_release_key", "bad_bundle_pin", "bad_root_pin",
                     "traffic_control", "stale_credentials", "tampered_download", "trial_token_rollback",
                     "outdated_bundle_resume", "live_enrollment_rejected"):
            record(guest("case", name))
        for point in ("prepared", "trial", "trial_token", "corrupt_backup"):
            armed = guest("arm", point)
            assert armed["armed"] == point
            stop(VM)
            boot(VM)
            record(guest("after", point))
    except Exception:
        print(json.dumps(guest("diagnose")), flush=True)
        raise
    finally:
        cleanup = guest("halt")
    build_info = json.loads((WORK / "enrollment-build.json").read_text())
    assert source_hashes() == build_info["source_sha256"], "source changed; discard this run"
    assert len(results) == 17 and all(item["passed"] for item in results)
    result = {"complete": True, "scenarios": results, "cleanup": cleanup, "build": build_info}
    (WORK / "enrollment-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
