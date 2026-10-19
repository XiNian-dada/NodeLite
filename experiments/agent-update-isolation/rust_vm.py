#!/usr/bin/env python3
"""Build under compiler ~/NodeLite and connect signed Rust download to fixed guest units."""

import json
import hashlib
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile

from rust_repository import prepare as prepare_repository
from vm import WORK, command, current_vm, ssh_args

HERE = WORK.parents[1] / "experiments/agent-update-isolation"


def sources():
    project = WORK.parents[1]
    paths = [project / "Cargo.toml", project / "Cargo.lock"]
    for name in ("nodelite-updater", "nodelite-agent", "nodelite-proto"):
        paths.extend((project / name / "src").rglob("*.rs"))
        paths.append(project / name / "Cargo.toml")
    paths.extend((project / "nodelite-updater/systemd").glob("*"))
    paths.extend(HERE.glob("rust*.py"))
    paths.extend([HERE / "tuf_repo.py", HERE / "vm.py", HERE / "fixture/src/main.rs"])
    return {str(path.relative_to(project)): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}


def guest(*args):
    arguments = ("sudo", "env", "PYTHONPATH=/usr/local/lib/nlup-lab/python-libs", "python3",
                 "/home/lab/agent-update-isolation/rust_guest.py", *args)
    result = subprocess.run([str(arg) for arg in ssh_args(current_vm())] + [
        " ".join(shlex.quote(arg) for arg in arguments)], capture_output=True, text=True, timeout=180)
    if result.returncode:
        raise RuntimeError(result.stderr[-4000:])
    return json.loads(result.stdout.strip().splitlines()[-1])


def prepare():
    before = sources()
    binaries = WORK / "rust-binaries"
    binaries.mkdir(mode=0o700, exist_ok=True)
    for name, version in (("agent-old", "rust-old"), ("agent-new", "rust-new")):
        env = {**os.environ, "NODELITE_BUILD_VERSION": version, "CARGO_BUILD_JOBS": "4"}
        command(["cargo", "build", "-p", "nodelite-agent", "--release", "--target", "x86_64-unknown-linux-musl", "--locked"], env=env)
        shutil.copyfile("target/x86_64-unknown-linux-musl/release/nodelite-agent", binaries / name)
        (binaries / name).chmod(0o755)
    command(["cargo", "build", "-p", "nodelite-updater", "--release", "--locked"],
            env={**os.environ, "CARGO_BUILD_JOBS": "4"})
    shutil.copyfile("target/release/nodelite-updater", binaries / "nodelite-updater")
    (binaries / "nodelite-updater").chmod(0o755)
    command(["cargo", "build", "-p", "nodelite-updater", "--release", "--features", "lab-faults", "--locked"],
            env={**os.environ, "CARGO_BUILD_JOBS": "4"})
    shutil.copyfile("target/release/nodelite-updater", binaries / "nodelite-updater-faults")
    (binaries / "nodelite-updater-faults").chmod(0o755)
    command(["cargo", "build", "--manifest-path", str(HERE / "fixture/Cargo.toml"), "--release", "--target", "x86_64-unknown-linux-musl", "--locked"],
            env={**os.environ, "CARGO_BUILD_JOBS": "4"})
    fixture = (HERE / "fixture/target/x86_64-unknown-linux-musl/release/nodelite-update-lab-fixture").read_bytes()
    with tempfile.TemporaryDirectory(prefix="rust-updater-", dir=WORK) as temporary:
        root = Path(temporary) / "rust-updater-input"
        root.mkdir()
        prepare_repository(root, binaries, fixture)
        shutil.copytree(WORK.parents[1] / "nodelite-updater/systemd", root / "systemd")
        command(["python3", HERE / "vm.py", "push", root, "/home/lab/"])
    command(["python3", HERE / "vm.py", "push", binaries, "/home/lab/"])
    command(["python3", HERE / "vm.py", "push", HERE, "/home/lab/"])
    assert sources() == before, "sources changed during the build; discard this run"
    (binaries / "build.json").write_text(json.dumps({"source_sha256": before,
        "agent_target": "x86_64-unknown-linux-musl", "updater_target": "native Linux GNU",
        "fault_hooks": "separate binary, disabled in normal binary"}, indent=2) + "\n")


def main():
    if Path.cwd().resolve() != WORK.parents[1].resolve() or os.geteuid() == 0:
        raise RuntimeError("run as compiler from ~/NodeLite only")
    output = WORK / "rust-updater-evidence.json"
    output.unlink(missing_ok=True)
    prepare()
    try:
        print(json.dumps(guest("enroll")), flush=True)
        result = guest("activate")
        print(json.dumps(result), flush=True)
        output.write_text(json.dumps({"complete": True, "scenarios": [result]}, indent=2) + "\n")
    except Exception:
        print(json.dumps(guest("diagnose")), flush=True)
        raise
    finally:
        guest("halt")


if __name__ == "__main__":
    main()
