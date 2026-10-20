"""Fixed installation boundary and bounded systemd operations for administrator migration."""

import grp
import json
import os
from pathlib import Path
import pwd
import stat
import subprocess
import time

from tools.nodelite_release.io import LIMIT, atomic, decode, digest, read, sync

HOME = Path("/var/lib/nodelite-updater")
POLICY = Path("/etc/nodelite-updater")
MIGRATION = Path("/var/lib/nodelite-updater-migration")
AGENT = "nodelite-agent.service"
UNIT = Path("/etc/systemd/system") / AGENT
CONFIG = Path("/var/lib/nodelite-agent/agent.toml")
LEGACY = Path("/usr/local/bin/nodelite-agent")
UPDATER = Path("/usr/local/bin/nodelite-updater")
BASELINE = Path("/usr/local/lib/nodelite-enrollment-agent")
LIB = Path("/usr/local/lib/nodelite-operator")
RECOVERY = "nodelite-updater-migration-recover.service"
DROPIN = Path("/etc/systemd/system/nodelite-agent.service.d/10-enrollment-recovery.conf")
UNITS = {AGENT, "nodelite-agent-preflight.service"}
UNITS.update("nodelite-updater-" + name for name in ("apply.service", "boot.service", "reconcile.service", "reconcile.timer",
    "preflight.service", "preflight.path", "stop.service", "stop.path", "restart.service", "restart.path"))
ENABLED = ("nodelite-updater-reconcile.timer", "nodelite-updater-preflight.path", "nodelite-updater-stop.path", "nodelite-updater-restart.path")


class PreflightError(RuntimeError):
    def __init__(self):
        super().__init__("Agent preflight rejected before stopping the running Agent; signed updates require "
                         "report_interval_secs within 1..10; check journalctl -u nodelite-agent-preflight.service")


def command(*args, check=True, timeout=40):
    result = subprocess.run([str(arg) for arg in args], capture_output=True, text=True, timeout=timeout,
                            env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C.UTF-8"})
    if check and result.returncode:
        raise RuntimeError("fixed administrator command failed: " + Path(args[0]).name)
    return result


def systemctl(*args, check=True):
    return command("/usr/bin/systemctl", *args, check=check)


def properties():
    result = systemctl("show", AGENT, "--property=MainPID,InvocationID,ActiveState,ControlGroup,User,Group,AmbientCapabilities,CapabilityBoundingSet,DropInPaths,FragmentPath,ExecStartPre,ExecStartPost,UnitFileState")
    if len(result.stdout) > 16384:
        raise ValueError("oversized service properties")
    values = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
    # systemd 249 omits empty execution arrays rather than printing an empty assignment.
    for key in ("ExecStartPre", "ExecStartPost"):
        values.setdefault(key, "")
    return values


def protected(path, directory=False):
    path = Path(path)
    for parent in path.parents:
        info = parent.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or info.st_mode & 0o022:
            raise ValueError("administrator path ancestry is not protected")
    info = path.lstat()
    if info.st_uid != 0 or info.st_mode & 0o022 or directory and not stat.S_ISDIR(info.st_mode):
        raise ValueError("administrator path ownership or type rejected")
    if not directory:
        read(path, LIMIT)


def root_guard():
    if os.geteuid() != 0 or not Path("/sys/fs/cgroup/cgroup.controllers").is_file() \
            or Path("/.dockerenv").exists() or Path("/run/systemd/container").exists():
        raise ValueError("migration requires local root on non-container systemd/cgroup-v2 Linux")
    for path in Path(__file__).parent.glob("*.py"):
        protected(path)


def inspect_legacy():
    for path in (UNIT, LEGACY):
        protected(path)
    user = pwd.getpwnam("nodelite-agent")
    for path, is_directory, mode in ((CONFIG.parent, True, 0o700), (CONFIG, False, 0o600)):
        info = path.lstat()
        if info.st_uid != user.pw_uid or stat.S_IMODE(info.st_mode) != mode \
                or is_directory != stat.S_ISDIR(info.st_mode):
            raise ValueError("legacy configuration must retain its dedicated owner and mode")
    info = CONFIG.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_size > 1024 * 1024:
        raise ValueError("unsafe legacy configuration inode")
    process = properties()
    expected = {"User": "nodelite-agent", "Group": "nodelite-agent", "AmbientCapabilities": "", "CapabilityBoundingSet": "",
                "DropInPaths": "", "FragmentPath": str(UNIT), "ExecStartPre": "", "ExecStartPost": "", "UnitFileState": "enabled"}
    if any(process.get(key) != value for key, value in expected.items()) \
            or process.get("ActiveState") != "active" or int(process.get("MainPID", "0")) <= 0:
        raise ValueError("only an enabled, healthy default capability-free installation is supported")
    unit = read(UNIT, 16384).decode()
    starts = [line for line in unit.splitlines() if line.startswith("ExecStart=")]
    if starts != [f"ExecStart={LEGACY} --config {CONFIG}"] \
            or "NODELITE_AGENT_TRAFFIC_CONTROL=1" in unit or "EnvironmentFile=" in unit:
        raise ValueError("custom launch command, environment file or traffic control is unsupported")
    # Released Agents expose their real build identity through sample-once, not --version.
    sample = command("/usr/sbin/runuser", "-u", "nodelite-agent", "--", "/usr/bin/env", "RUST_LOG=off",
                     LEGACY, "--config", CONFIG, "--sample-once").stdout
    if len(sample) > 1024 * 1024:
        raise ValueError("oversized legacy sample")
    version = decode(sample)["identity"]["agent_version"]
    import re
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:[-+][A-Za-z0-9._-]+)?", version)
    if match is None or tuple(map(int, match.groups())) < (3, 0, 4):
        raise ValueError("migrate only after a durable-token-compatible Agent version >= 3.0.4")
    if "security.capability" in os.listxattr(LEGACY) or LEGACY.stat().st_mode & 0o6000:
        raise ValueError("privileged legacy executable rejected")
    if os.readlink(f"/proc/{process['MainPID']}/exe") != str(LEGACY):
        raise ValueError("running legacy executable does not match the supported path")
    return process


def stop_barrier():
    process = properties()
    if int(process.get("MainPID", "0")):
        systemctl("stop", AGENT)
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        process = properties()
        group = process.get("ControlGroup", "")
        if group and (not group.startswith("/system.slice/") or ".." in group.split("/")):
            raise ValueError("unexpected service cgroup")
        path = Path("/sys/fs/cgroup") / group.lstrip("/") / "cgroup.procs"
        empty = not group or not path.exists() or not path.read_text().strip()
        if process.get("MainPID") == "0" and empty:
            return
        time.sleep(0.1)
    raise RuntimeError("legacy stop barrier did not complete")


def save(state):
    atomic(MIGRATION / "journal.json", (json.dumps(state, sort_keys=True) + "\n").encode(), 0o600)


def config_barrier():
    # Never snapshot/restore credentials: the last Agent writer is authoritative.
    fd = os.open(CONFIG, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or stat.S_IMODE(info.st_mode) != 0o600:
            raise ValueError("configuration inode changed unsafely")
        os.fsync(fd)
    finally:
        os.close(fd)
    sync(CONFIG.parent)


def directories(path, user="root", group="root", mode=0o755):
    path = Path(path)
    path.mkdir(mode=mode)
    os.chown(path, pwd.getpwnam(user).pw_uid, grp.getgrnam(group).gr_gid)
    path.chmod(mode)
    sync(path.parent)
