#!/usr/bin/env python3
"""Isolate installer downloads and service control; HTTP, archives and SQLite remain real."""

import fcntl
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(os.environ["NODELITE_INSTALL_TEST_ROOT"])
COMMAND = Path(sys.argv[0]).name
ARGS = sys.argv[1:]


def event(value):
    with (ROOT / "events").open("a") as output:
        output.write(value + "\n")


def pid():
    try:
        value = int((ROOT / "pid").read_text())
        os.kill(value, 0)
        if (ROOT / "running").read_text() == str(value):
            return value
    except (FileNotFoundError, ProcessLookupError):
        pass
    return 0


def systemctl():
    command = ARGS[0]
    event("systemctl-" + command)
    if command == "stop":
        value = pid()
        if value:
            os.kill(value, signal.SIGTERM)
            deadline = time.monotonic() + 5
            while pid() and time.monotonic() < deadline:
                time.sleep(0.02)
            return int(bool(pid()))
        return 0
    if command == "is-active":
        return 0 if pid() else 3
    if command == "show":
        if "--property=MainPID" in ARGS:
            print(pid())
        else:
            print("active" if pid() else "inactive")
        return 0
    if command == "start":
        if pid():
            return 0
        unit = dict(line.split("=", 1) for line in Path(os.environ["NODELITE_INSTALL_TEST_UNIT"]).read_text().splitlines() if "=" in line)
        with (ROOT / "server.log").open("a") as output:
            process = subprocess.Popen(shlex.split(unit["ExecStart"]), cwd=unit["WorkingDirectory"],
                                       stdout=output, stderr=output, start_new_session=True)
        (ROOT / "pid").write_text(str(process.pid))
        deadline = time.monotonic() + 2
        while process.poll() is None and not pid() and time.monotonic() < deadline:
            time.sleep(0.01)
        return 0
    if command in {"enable", "disable", "daemon-reload"}:
        return 0
    raise RuntimeError(f"unexpected systemctl command: {ARGS}")


def curl():
    url = next(arg for arg in ARGS if arg.startswith(("http://", "https://")))
    if url.startswith("http://127.0.0.1:"):
        os.execv(os.environ["NODELITE_INSTALL_REAL_CURL"], ["curl", *ARGS])
    if not url.startswith("https://example.invalid/releases/v9.9.9/"):
        raise RuntimeError(f"unexpected release URL: {url}")
    name = url.rsplit("/", 1)[1]
    event("download-" + name)
    shutil.copyfile(ROOT / "release" / name, ARGS[ARGS.index("-o") + 1])
    return 0


def archive():
    scenario = os.environ["NODELITE_INSTALL_TEST_SCENARIO"]
    if scenario == "backup_fail" and ARGS[0].startswith("-c"):
        event("injected-backup-failure")
        return 1
    if scenario == "restore_fail" and ARGS[0].startswith("-x"):
        event("injected-restore-failure")
        return 1
    os.execv(os.environ["NODELITE_INSTALL_REAL_TAR"], ["tar", *ARGS])


def install():
    mode = 0o755
    files = []
    index = 0
    while index < len(ARGS):
        arg = ARGS[index]
        if arg in {"-o", "-g", "-m"}:
            if arg == "-m":
                mode = int(ARGS[index + 1], 8)
            index += 2
        else:
            files.append(arg)
            index += 1
    shutil.copyfile(*files)
    os.chmod(files[1], mode)
    return 0


def timeout():
    args = [arg for arg in ARGS if not arg.startswith("--kill-after=")]
    seconds = float(args.pop(0))
    with subprocess.Popen(args, start_new_session=True) as process:
        try:
            return process.wait(timeout=seconds)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            return 124


if COMMAND == "systemctl":
    sys.exit(systemctl())
if COMMAND == "curl":
    sys.exit(curl())
if COMMAND == "tar":
    sys.exit(archive())
if COMMAND == "install":
    sys.exit(install())
if COMMAND == "timeout":
    sys.exit(timeout())
if COMMAND == "flock":
    try:
        fcntl.flock(int(ARGS[-1]), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        sys.exit(1)
elif COMMAND == "id":
    assert ARGS == ["-u"]
    print(0)
elif COMMAND == "chown":
    pass
else:
    raise RuntimeError(f"unexpected fixture command: {COMMAND}")
