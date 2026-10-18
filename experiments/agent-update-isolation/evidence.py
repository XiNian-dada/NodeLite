#!/usr/bin/env python3
"""Collect only allowlisted public lab results; never read credential files."""

import json
import shlex
import subprocess

from vm import WORK, ssh_args


def guest(*args):
    result = subprocess.run([str(part) for part in ssh_args(WORK / "vm-v1")] +
                            [" ".join(shlex.quote(part) for part in args)],
                            check=True, capture_output=True, text=True, timeout=20)
    return result.stdout


def main():
    files = ["/var/lib/nlup-lab", "/var/lib/nlup-lab/code", "/var/lib/nlup-lab/private",
             "/var/lib/nlup-lab/agent", "/var/lib/nlup-lab/inbox", "/var/lib/nlup-lab/status",
             "/var/lib/nlup-lab/bridge", "/usr/local/lib/nlup-lab/updater.py",
             "/etc/nlup-lab/release.pub", "/etc/systemd/system/nlup-lab-agent.service"]
    units = ["agent.service", "preflight.service", "request.service", "request.path",
             "reconcile.service", "reconcile.timer", "boot.service", "restart-bridge.service",
             "restart-bridge.path", "preflight-bridge.service", "preflight-bridge.path"]
    host_units = subprocess.run(["systemctl", "list-unit-files", "nlup-lab-*", "--no-legend"],
                                capture_output=True, text=True)
    if host_units.returncode not in {0, 1} or host_units.stderr.strip():
        raise RuntimeError("cannot establish compiler host unit inventory")
    result = {
        "guest_kernel": guest("uname", "-r").strip(),
        "guest_systemd": guest("systemctl", "--version").splitlines()[0],
        "virtualization": guest("systemd-detect-virt", "--vm").strip(),
        "permissions": guest("sudo", "stat", "-c", "%a %U:%G %n", *files).splitlines(),
        "installed_code_hashes": guest("sha256sum", "/usr/local/lib/nlup-lab/updater.py",
                                      "/usr/local/lib/nlup-lab/lab_io.py").splitlines(),
        "unit_verify": guest("sudo", "systemd-analyze", "verify", *["nlup-lab-" + name for name in units]),
        "units": {name: guest("cat", "/etc/systemd/system/nlup-lab-" + name) for name in units},
        "scenarios": json.loads(guest("sudo", "cat", "/home/lab/results.json")),
        "host_lab_units": host_units.stdout,
        "host_lab_unit_query_exit": host_units.returncode,
    }
    (WORK / "evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"scenarios_passed": len(result["scenarios"]),
                      "host_lab_units_empty": not result["host_lab_units"].strip(),
                      "guest_systemd": result["guest_systemd"]}))


if __name__ == "__main__":
    main()
