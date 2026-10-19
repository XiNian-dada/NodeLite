#!/usr/bin/env python3
"""Run a disposable KVM guest; no host units, users, mounts or privileged container."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import time

WORK = Path.home() / "NodeLite/output/agent-update-prototype"
SNAP = Path("/snap/lxd/current")
IMAGE = WORK / "jammy-server-cloudimg-amd64.img"


def current_vm():
    selector = os.environ.get("NODELITE_LAB_VM", "legacy")
    if selector not in {"legacy", "enrollment"}:
        raise RuntimeError("only the two fixed disposable lab VMs are supported")
    return WORK / ("vm-enrollment-v1" if selector == "enrollment" else "vm-v1")


def command(args, **kwargs):
    return subprocess.run([str(value) for value in args], check=True, **kwargs)


def runtime_env():
    env = os.environ.copy()
    paths = [WORK / "tools/lib", WORK / "tools/unpacked/usr/lib/x86_64-linux-gnu",
             SNAP / "lib", SNAP / "lib/x86_64-linux-gnu",
             SNAP / "lib/x86_64-linux-gnu/ceph", SNAP / "usr/lib/x86_64-linux-gnu"]
    env["LD_LIBRARY_PATH"] = ":".join(str(path) for path in paths)
    env["QEMU_MODULE_DIR"] = str(WORK / "tools/unpacked/usr/lib/x86_64-linux-gnu/qemu")
    return env


def ssh_args(directory):
    return ["ssh", "-i", directory / "guest_key", "-p", "22239",
            "-o", "BatchMode=yes", "-o", "ConnectTimeout=2",
            "-o", "ForwardAgent=no", "-o", "ClearAllForwardings=yes", "-o", "IdentitiesOnly=yes",
            "-o", "StrictHostKeyChecking=accept-new",
            "-o", f"UserKnownHostsFile={directory / 'known_hosts'}", "lab@127.0.0.1"]


def stop(directory):
    pid_file = directory / "qemu.pid"
    if not pid_file.exists():
        return
    pid = int(pid_file.read_text())
    cmdline = Path(f"/proc/{pid}/cmdline")
    if cmdline.exists():
        if str(directory / "disk.qcow2").encode() not in cmdline.read_bytes():
            raise RuntimeError("refusing to signal an unrelated process")
        os.kill(pid, signal.SIGKILL)
        for _ in range(100):
            if not cmdline.exists() or not cmdline.read_bytes():
                break
            time.sleep(0.05)
    pid_file.unlink(missing_ok=True)


def prepare(directory):
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    if not (directory / "guest_key").exists():
        command(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", directory / "guest_key"])
    if not (directory / "disk.qcow2").exists():
        lines = (WORK / "SHA256SUMS").read_text().splitlines()
        expected = next(line.split()[0] for line in lines if line.split()[-1].lstrip("*") == IMAGE.name)
        with IMAGE.open("rb") as stream:
            actual = hashlib.file_digest(stream, "sha256").hexdigest() if hasattr(hashlib, "file_digest") else None
            if actual is None:
                digest = hashlib.sha256()
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(block)
                actual = digest.hexdigest()
        if actual != expected:
            raise RuntimeError("Ubuntu image checksum mismatch")
        command([SNAP / "bin/qemu-img", "create", "-f", "qcow2", "-F", "qcow2",
                 "-b", IMAGE, directory / "disk.qcow2", "8G"], env=runtime_env())
        (directory / "image-identity.json").write_text(json.dumps({"sha256": actual, "source":
            "https://cloud-images.ubuntu.com/jammy/current/jammy-server-cloudimg-amd64.img"}) + "\n")
    seed = directory / "seed"
    seed.mkdir(exist_ok=True)
    public = (directory / "guest_key.pub").read_text().strip()
    config = """#cloud-config
users:
  - name: lab
    groups: [sudo]
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    ssh_authorized_keys:
      - %s
ssh_pwauth: false
disable_root: true
write_files:
  - path: /etc/nlup-lab-guest
    permissions: '0444'
    content: isolated-prototype-v1
""" % public
    (seed / "user-data").write_text(config)
    (seed / "meta-data").write_text("instance-id: nlup-lab-v1\nlocal-hostname: nlup-lab\n")
    iso = next((WORK / "tools").glob("genisoimage_*.deb"))
    tools = WORK / "tools/unpacked"
    if not tools.exists():
        command(["dpkg-deb", "-x", iso, tools])
    command([tools / "usr/bin/genisoimage", "-quiet", "-output", directory / "seed.iso",
             "-volid", "cidata", "-joliet", "-rock", seed / "user-data", seed / "meta-data"])


def boot(directory):
    args = [WORK / "tools/unpacked/usr/bin/qemu-system-x86_64", "-enable-kvm", "-machine", "q35",
             "-cpu", "host", "-smp", "2", "-m", "1024", "-display", "none",
             "-L", SNAP / "share/qemu", "-bios", SNAP / "share/qemu/bios-256k.bin",
             "-drive", f"file={directory / 'disk.qcow2'},format=qcow2,if=virtio,cache=none",
             "-drive", f"file={directory / 'seed.iso'},media=cdrom,readonly=on",
             "-netdev", "user,id=net0,restrict=on,hostfwd=tcp:127.0.0.1:22239-:22",
             "-device", "virtio-net-pci,netdev=net0", "-serial", f"file:{directory / 'serial.log'}",
             "-pidfile", directory / "qemu.pid", "-daemonize"]
    for _ in range(30):
        result = subprocess.run([str(arg) for arg in args], env=runtime_env(), capture_output=True, text=True)
        if result.returncode == 0:
            break
        if 'Failed to get "write" lock' not in result.stderr:
            raise RuntimeError(result.stderr)
        time.sleep(0.2)
    else:
        raise RuntimeError("previous QEMU disk lock did not release; refusing concurrent disk access")
    for _ in range(90):
        result = subprocess.run([str(value) for value in ssh_args(directory)] + ["true"],
                                capture_output=True, timeout=4)
        if result.returncode == 0:
            return
        time.sleep(1)
    raise RuntimeError("guest SSH did not become ready; inspect serial.log")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=["start", "stop", "powercut", "exec", "push", "pull"])
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    directory = current_vm()
    if args.action == "start":
        prepare(directory)
        boot(directory)
    elif args.action in {"stop", "powercut"}:
        stop(directory)
    elif args.action == "exec":
        command(ssh_args(directory) + [" ".join(shlex.quote(arg) for arg in args.arguments)])
    else:
        source, destination = args.arguments
        transport = " ".join(shlex.quote(str(arg)) for arg in ssh_args(directory)[:-1])
        remote = "lab@127.0.0.1:"
        if args.action == "push":
            command(["rsync", "-r", "--exclude=__pycache__", "--exclude=*.pyc",
                     "-e", transport, source, remote + destination])
        else:
            command(["rsync", "-r", "-e", transport, remote + source, destination])


if __name__ == "__main__":
    main()
