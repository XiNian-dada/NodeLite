#!/usr/bin/env python3
"""Abrupt guest power loss at durable checkpoints; never reboots the compiler host."""

import json
import shlex
import subprocess

from vm import WORK, boot, ssh_args, stop


def guest(directory, *arguments):
    command = ["sudo", "python3", "/home/lab/agent-update-isolation/test_guest.py", *arguments]
    result = subprocess.run([str(part) for part in ssh_args(directory)] +
                            [" ".join(shlex.quote(part) for part in command)],
                            check=True, capture_output=True, text=True, timeout=45)
    return json.loads(result.stdout.strip().splitlines()[-1])


def main():
    directory = WORK / "vm-v1"
    results = []
    for point in ("prepared", "stopping", "trial_journal", "active_pointer"):
        before = guest(directory, "arm", point)
        stop(directory)
        boot(directory)
        after = guest(directory, "after-powercut")
        result = {"checkpoint": point, "before": before, "after": after}
        results.append(result)
        (WORK / "powercuts.json").write_text(json.dumps(results, indent=2) + "\n")
        print(json.dumps(result), flush=True)
    guest(directory, "arm-corrupt")
    stop(directory)
    boot(directory)
    result = {"checkpoint": "corrupt_journal", "after": guest(directory, "after-corrupt")}
    results.append(result)
    (WORK / "powercuts.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
