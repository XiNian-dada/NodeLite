#!/usr/bin/env python3
"""Build on compiler, run real Agent tests inside the already-running isolated VM."""

import hashlib
import json
import os
from pathlib import Path
import pwd
import shlex
import shutil

from vm import WORK, command, ssh_args

PROJECT = Path.home() / "NodeLite"
HERE = PROJECT / "experiments/agent-update-isolation"
TEST = "integration_tests::updater_handoff::real_agent_token_handoff_in_disposable_vm"


def guest(*args):
    return command(ssh_args(WORK / "vm-v1") + [" ".join(shlex.quote(str(arg)) for arg in args)])


def build():
    directory = WORK / "real-binaries"
    directory.mkdir(mode=0o700, exist_ok=True)
    for version in ("old", "new"):
        env = os.environ.copy()
        env["NODELITE_BUILD_VERSION"] = "lab-" + version
        command(["cargo", "build", "-p", "nodelite-agent", "--release", "--locked"], cwd=PROJECT, env=env)
        shutil.copy2(PROJECT / "target/release/nodelite-agent", directory / f"agent-{version}")
    env = os.environ.copy()
    env["NODELITE_SKIP_WEB_BUILD"] = "1"
    result = command(["cargo", "test", "-p", "nodelite-server", "--lib", "--no-run", "--locked",
                      "--message-format=json"], cwd=PROJECT, env=env, capture_output=True, text=True)
    artifacts = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
    executables = [item["executable"] for item in artifacts if item.get("reason") == "compiler-artifact"
                   and item.get("executable") and item["target"]["name"] == "nodelite_server"]
    if len(executables) != 1:
        raise RuntimeError("cannot uniquely identify the freshly built server test binary")
    shutil.copy2(executables[0], directory / "real-test-runner")
    command(["strip", "--strip-debug", directory / "real-test-runner"])
    listing = command([directory / "real-test-runner", "--list", "--format", "terse"],
                      capture_output=True, text=True).stdout.splitlines()
    if TEST + ": test" not in listing:
        raise RuntimeError("matching Rust workspace/test-module hook was not synced; refusing an empty test run")
    return directory


def main():
    if pwd.getpwuid(os.geteuid()).pw_name != "compiler" or Path.cwd().resolve() != PROJECT.resolve() \
            or 'name = "nodelite-agent"' not in (PROJECT / "nodelite-agent/Cargo.toml").read_text():
        raise RuntimeError("run from the verified compiler ~/NodeLite checkout only")
    directory = build()
    command(["python3", HERE / "vm.py", "push", HERE, "/home/lab/"])
    command(["python3", HERE / "vm.py", "push", directory, "/home/lab/"])
    guest("sudo", "python3", "/home/lab/agent-update-isolation/real_guest.py", "halt")
    guest("rm", "-f", "--", "/home/lab/real-agent-results.json")
    for name in ("updater.py", "lab_io.py"):
        guest("sudo", "install", "-o", "root", "-g", "root", "-m", "0555",
              f"/home/lab/agent-update-isolation/{name}", f"/usr/local/lib/nlup-lab/{name}")
    hashes = command(ssh_args(WORK / "vm-v1") +
                     ["sha256sum /usr/local/lib/nlup-lab/updater.py /usr/local/lib/nlup-lab/lab_io.py"],
                     capture_output=True, text=True).stdout.splitlines()
    try:
        guest("sudo", "/home/lab/real-binaries/real-test-runner", "--ignored", "--exact",
              TEST, "--nocapture")
        command(["python3", HERE / "vm.py", "pull", "/home/lab/real-agent-results.json",
                 WORK / "real-agent-results.json"])
        evidence = {"installed_worker_hashes": hashes,
                    "binaries": {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                                 for path in directory.iterdir()},
                    "scenarios": json.loads((WORK / "real-agent-results.json").read_text())}
        (WORK / "real-evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    finally:
        guest("sudo", "python3", "/home/lab/agent-update-isolation/real_guest.py", "halt")


if __name__ == "__main__":
    main()
