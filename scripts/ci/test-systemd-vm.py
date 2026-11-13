#!/usr/bin/env python3
"""Run privileged acceptance inside a disposable guest, without a host Docker socket."""

import argparse
import json
import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path


def run(binary, base_image, report):
    root = Path(__file__).resolve().parents[2]
    report.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="nodelite-systemd-") as temp:
        work = Path(temp)
        payload = work / "payload"
        payload.mkdir()
        shutil.copytree(root / "scripts", payload / "scripts")
        shutil.copy2(binary, payload / "agent-tests")
        guest = '''#!/bin/bash
set -euo pipefail
trap 'code=$?; echo "NODELITE_SYSTEMD_RESULT=$code" >/dev/ttyS0; poweroff -f' EXIT
export DEBIAN_FRONTEND=noninteractive
export http_proxy=http://172.29.84.1:17892
export https_proxy=http://172.29.84.1:17892
export no_proxy=localhost,127.0.0.1
apt-get -o Acquire::Retries=2 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 update
apt-get -o Acquire::Retries=2 install -y --no-install-recommends python3 curl iproute2 libssl3t64
mkdir -p /mnt/payload /opt/nodelite-test
mount -o ro /dev/disk/by-label/NODELITE /mnt/payload
cp -a /mnt/payload/. /opt/nodelite-test/
cd /opt/nodelite-test
chmod 0755 agent-tests
test "$(cat /proc/1/comm)" = systemd
python3 scripts/test-install-server-upgrade.py --systemd crash not_ready wrong_version success migrate_fail corrupt_backup restore_fail signal
sh scripts/test-traffic-control-systemd.sh /opt/nodelite-test/agent-tests
'''
        seed = work / "seed"
        seed.mkdir()
        (seed / "meta-data").write_text("instance-id: nodelite-systemd-test\nlocal-hostname: nodelite-test\n")
        # No password or host credential is supplied to the isolated guest.
        cloud = {"ssh_pwauth": False, "disable_root": True,
                 "write_files": [{"path": "/root/run-nodelite-tests.sh", "permissions": "0700", "content": guest}],
                 "runcmd": [["bash", "/root/run-nodelite-tests.sh"]]}
        (seed / "user-data").write_text("#cloud-config\n" + json.dumps(cloud))
        for name, directory, label in [("payload", payload, "NODELITE"), ("seed", seed, "cidata")]:
            subprocess.run(["genisoimage", "-quiet", "-output", str(work / f"{name}.iso"),
                            "-volid", label, "-joliet", "-rock", str(directory)], check=True)
        overlay = work / "guest.qcow2"
        subprocess.run(["qemu-img", "create", "-f", "qcow2", "-F", "qcow2", "-b",
                        str(base_image.resolve()), str(overlay), "5G"], check=True)
        log = report / "systemd-serial.log"
        command = ["qemu-system-x86_64", "-machine", "q35,accel=tcg", "-cpu", "max", "-smp", "2",
                   "-m", "1024", "-display", "none", "-monitor", "none", "-serial", "stdio", "-no-reboot",
                   "-drive", f"file={overlay},if=virtio,format=qcow2",
                   "-drive", f"file={work / 'seed.iso'},if=virtio,format=raw,readonly=on",
                   "-drive", f"file={work / 'payload.iso'},if=virtio,format=raw,readonly=on",
                   "-netdev", "user,id=net0", "-device", "virtio-net-pci,netdev=net0"]
        with log.open("w") as stream:
            started = time.monotonic()
            process = subprocess.Popen(command, stdout=stream, stderr=subprocess.STDOUT)
            try:
                while process.poll() is None:
                    if time.monotonic() - started > 1200:
                        raise TimeoutError("systemd guest exceeded 20 minutes")
                    time.sleep(5)
                returncode = process.returncode
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
        contents = log.read_text(errors="replace")
        if returncode or "NODELITE_SYSTEMD_RESULT=0" not in contents:
            print(contents[-12000:])
            raise SystemExit("systemd guest failed; inspect systemd-serial.log")
        print("Real systemd upgrade and traffic-control acceptance passed in a disposable QEMU guest")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--image", type=Path, default=Path("/opt/nodelite-ci/debian-systemd.qcow2"))
    parser.add_argument("--report", type=Path, default=Path(os.environ.get("RUNNER_TEMP", "/tmp")) / "systemd-report")
    args = parser.parse_args()
    run(args.binary, args.image, args.report)
