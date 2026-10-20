"""Journaled legacy-to-signed handoff, with a root-protected boot recovery and no config restore."""

import grp
import json
import os
from pathlib import Path
import pwd
import secrets
import stat
import time

from tools.nodelite_release.io import LIMIT, atomic, create, decode, digest, json_bytes, read, sync

from . import bundle
from .common import (AGENT, BASELINE, CONFIG, DROPIN, ENABLED, HOME, LEGACY, LIB, MIGRATION, POLICY,
                     RECOVERY, UNIT, UNITS, UPDATER, command, config_barrier, directories, inspect_legacy,
                     properties, protected, root_guard, save, stop_barrier, systemctl)


def accounts():
    for name in ("nodelite-updater", "nodelite-read"):
        try:
            grp.getgrnam(name)
        except KeyError:
            command("/usr/sbin/groupadd", "--system", name)
    try:
        user = pwd.getpwnam("nodelite-updater")
        if user.pw_uid == 0 or user.pw_shell not in {"/usr/sbin/nologin", "/sbin/nologin", "/bin/false"}:
            raise ValueError("updater account collision")
    except KeyError:
        command("/usr/sbin/useradd", "--system", "--no-create-home", "--shell", "/usr/sbin/nologin",
                "--gid", "nodelite-updater", "nodelite-updater")
    for user in validate_accounts():
        command("/usr/sbin/usermod", "--append", "--groups", "nodelite-read", user)


def validate_accounts():
    account = pwd.getpwnam("nodelite-updater")
    if account.pw_uid == 0 or account.pw_shell not in {"/usr/sbin/nologin", "/sbin/nologin", "/bin/false"}:
        raise ValueError("updater account collision")
    if not set(grp.getgrnam("nodelite-read").gr_mem) <= {"nodelite-agent", "nodelite-updater"}:
        raise ValueError("shared read group contains unrelated accounts")
    for user in ("nodelite-agent", "nodelite-updater"):
        account = pwd.getpwnam(user)
        own = grp.getgrnam(user)
        permitted = {own.gr_gid, grp.getgrnam("nodelite-read").gr_gid}
        if own.gr_gid == 0 or account.pw_gid != own.gr_gid \
                or not set(os.getgrouplist(user, account.pw_gid)) <= permitted \
                or len([entry for entry in pwd.getpwall() if entry.pw_uid == account.pw_uid]) != 1:
            raise ValueError("dedicated account has unexpected groups or shared UID")
        if not set(own.gr_mem) <= {user}:
            raise ValueError("dedicated group contains unrelated users")
    allowed = {"nodelite-agent", "nodelite-updater"}
    gids = {grp.getgrnam(name).gr_gid for name in (*allowed, "nodelite-read")}
    if 0 in gids or any(entry.pw_gid in gids and entry.pw_name not in allowed for entry in pwd.getpwall()):
        raise ValueError("dedicated groups are shared with another primary account")
    return allowed


def install_recovery():
    source = Path(__file__).resolve().parents[1]
    directories(LIB)
    directories(LIB / "tools")
    create(LIB / "tools/__init__.py", read(source / "__init__.py"), 0o555)
    for package, names in (("nodelite_release", ("__init__.py", "io.py")),
                           ("nodelite_migrate", ("__init__.py", "common.py", "bundle.py", "transaction.py", "audit.py", "installer.py", "__main__.py"))):
        directories(LIB / "tools" / package)
        for name in names:
            create(LIB / "tools" / package / name, read(source / package / name), 0o555)
    launcher = b'import sys\nsys.path.insert(0, "/usr/local/lib/nodelite-operator")\nfrom tools.nodelite_migrate.__main__ import main\nmain()\n'
    create(LIB / "recover.py", launcher, 0o555)
    unit = f'''[Unit]
Description=NodeLite first migration boot recovery (local administrator only)
After=local-fs.target
Before=nodelite-agent.service
[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/bin/python3 -I {LIB}/recover.py --recover
UMask=0077
NoNewPrivileges=yes
ProtectHome=yes
PrivateTmp=yes
[Install]
WantedBy=multi-user.target
'''
    create(UNIT.parent / RECOVERY, unit.encode())
    systemctl("daemon-reload")
    systemctl("enable", "--now", RECOVERY)


def ensure_layout(resuming):
    layout = ((HOME, "root", "root", 0o755),
            (HOME / "private", "nodelite-updater", "nodelite-updater", 0o700),
            (HOME / "code", "nodelite-updater", "nodelite-read", 0o750),
            (HOME / "code/objects", "nodelite-updater", "nodelite-read", 0o750),
            (HOME / "bridge", "root", "nodelite-updater", 0o770), (POLICY, "root", "root", 0o755))
    for path, user, group, mode in layout:
        if resuming and path.exists():
            info = path.lstat()
            if not stat.S_ISDIR(info.st_mode) or info.st_uid != pwd.getpwnam(user).pw_uid \
                    or info.st_gid != grp.getgrnam(group).gr_gid or stat.S_IMODE(info.st_mode) != mode:
                raise ValueError("existing migration directory is unsafe")
        else:
            directories(path, user, group, mode)


def stage_programs(files, policy, resuming):
    for path, value, mode, mutable in ((POLICY / "root.json", files["root.json"], 0o644, False),
            (UPDATER, files["nodelite-updater"], 0o555, False),
            (POLICY / "policy.toml", policy, 0o644, True), (BASELINE, files["nodelite-agent"], 0o555, True)):
        if resuming and path.exists():
            protected(path)
            if mutable:
                atomic(path, value, mode)
            elif read(path, LIMIT) != value:
                raise ValueError("privileged program or trust anchor changed")
        else:
            create(path, value, mode)


def provision(files, policy, resuming=False):
    accounts()
    ensure_layout(resuming)
    stage_programs(files, policy, resuming)
    started = MIGRATION / "trust-started"
    if resuming and started.exists() and not (HOME / "private/journal.json").is_file():
        raise ValueError("trust state is missing; never reinitialize during resume")
    if not started.exists():
        create(started, b"trust must never be reset\n", 0o600)
    command("/usr/sbin/runuser", "-u", "nodelite-updater", "-g", "nodelite-read", "-G", "nodelite-updater", "--",
            UPDATER, "verify-enrollment", "--baseline", BASELINE, timeout=65)
    for name in sorted(UNITS - {AGENT}):
        path, value = UNIT.parent / name, files["systemd/" + name]
        if resuming and path.exists():
            protected(path)
            if read(path) != value:
                raise ValueError("fixed unit changed")
        else:
            create(path, value)
    systemctl("daemon-reload")
    systemctl("reset-failed", "nodelite-agent-preflight.service", check=False)
    systemctl("start", "nodelite-agent-preflight.service")


def healthy(expected, timeout):
    start, first, invocation = time.monotonic(), None, None
    while time.monotonic() - start < timeout:
        try:
            process = properties()
            evidence = decode(read(Path("/run/nodelite-agent/update-health.json"), 4096))
            wall, boot = time.time() * 1000, time.clock_gettime(time.CLOCK_BOOTTIME) * 1000
            valid = process["ActiveState"] == "active" and evidence["schema"] == 1 \
                and evidence["pid"] == int(process["MainPID"]) and evidence["invocation"] == process["InvocationID"] \
                and evidence["executable_sha256"] == expected and 0 <= wall - evidence["collected_at_ms"] < 15000 \
                and 0 <= boot - evidence["sample_boottime_ms"] < 15000
            if valid:
                if invocation != process["InvocationID"]:
                    # This privileged one-time admin check is separate from unprivileged updater IPC.
                    with open(f"/proc/{process['MainPID']}/exe", "rb") as stream:
                        if digest(stream.read(LIMIT + 1)) != expected:
                            raise ValueError("migration process image mismatch")
                    invocation, first = process["InvocationID"], (time.monotonic(), evidence["collected_at_ms"])
                elif time.monotonic() - first[0] >= 5 and evidence["collected_at_ms"] - first[1] >= 5000:
                    return
            else:
                first, invocation = None, None
        except (OSError, ValueError, KeyError):
            first, invocation = None, None
        time.sleep(0.2)
    raise RuntimeError("signed migration baseline did not establish authenticated health")


def apply(path, bundle_sha256, root_sha256, metadata_url, targets_url, timeout=60):
    root_guard()
    manifest, files = bundle.inspect(path, bundle_sha256, root_sha256)
    policy = bundle.policy(manifest, files, metadata_url, targets_url)
    inspect_legacy()
    if manifest["minimum_sequence"] > 1 and digest(read(LEGACY, LIMIT)) != digest(files["nodelite-agent"]):
        raise ValueError("revoked legacy rollback: manually install the authorized current Agent first")
    paths = [HOME, POLICY, MIGRATION, UPDATER, BASELINE, LIB, UNIT.parent / RECOVERY]
    paths.extend(UNIT.parent / name for name in UNITS - {AGENT})
    if any(item.exists() or item.is_symlink() for item in paths):
        raise ValueError("an existing updater/migration installation requires explicit operator recovery")
    directories(MIGRATION, mode=0o700)
    create(MIGRATION / "old.service", read(UNIT, 16384), 0o600)
    create(MIGRATION / "old-agent", read(LEGACY, LIMIT), 0o600)
    create(MIGRATION / "receipt.json", json_bytes({"manifest": manifest,
        "metadata_url": metadata_url, "targets_url": targets_url}), 0o600)
    install_recovery()
    state = {"schema": 1, "phase": "prepared", "old_unit_sha256": digest(read(UNIT, 16384)),
             "old_binary_sha256": digest(read(LEGACY, LIMIT)), "bundle_sha256": bundle_sha256,
             "new_binary_sha256": digest(files["nodelite-agent"])}
    save(state)
    return handoff(state, files, policy, timeout)


def resume(path, bundle_sha256, root_sha256, metadata_url, targets_url, timeout=60):
    root_guard()
    manifest, files = bundle.inspect(path, bundle_sha256, root_sha256)
    policy = bundle.policy(manifest, files, metadata_url, targets_url)
    inspect_legacy()
    state = load_state()
    protected(MIGRATION / "receipt.json")
    receipt = decode(read(MIGRATION / "receipt.json", 16384))
    original = receipt["manifest"]
    if state["phase"] != "rolled_back" or (receipt["metadata_url"], receipt["targets_url"]) != (metadata_url, targets_url) \
            or any(manifest[name] != original[name] for name in ("channel", "target", "minimum_sequence")) \
            or any(value != original["files"][name] for name, value in manifest["files"].items() if name != "nodelite-agent"):
        raise ValueError("resume cannot replace privileged files, anchor, scope or original rollback floor")
    if digest(read(UNIT, 16384)) != state["old_unit_sha256"] \
            or digest(read(LEGACY, LIMIT)) != state["old_binary_sha256"]:
        raise ValueError("legacy recovery installation changed")
    if systemctl("is-active", RECOVERY, check=False).returncode:
        raise ValueError("start the fixed recovery unit before resuming")
    create(MIGRATION / ("attempt-" + secrets.token_hex(8) + ".json"), json_bytes(state), 0o600)
    state.update(phase="prepared", bundle_sha256=bundle_sha256, new_binary_sha256=digest(files["nodelite-agent"]))
    save(state)
    return handoff(state, files, policy, timeout, resuming=True)


def handoff(state, files, policy, timeout, resuming=False):
    try:
        provision(files, policy, resuming)
        state["phase"] = "stopping"
        save(state)
        stop_barrier()
        config_barrier()
        state["phase"] = "switching"
        save(state)
        if DROPIN.parent.exists():
            protected(DROPIN.parent, directory=True)
        else:
            directories(DROPIN.parent)
        create(DROPIN, f"[Unit]\nRequires={RECOVERY}\nAfter={RECOVERY}\n".encode())
        # A new unit must never survive a power cut without its fail-closed recovery dependency.
        atomic(UNIT, files["systemd/" + AGENT])
        systemctl("daemon-reload")
        command("/usr/bin/systemd-analyze", "verify", *(UNIT.parent / name for name in sorted(UNITS)))
        state["phase"] = "trial"
        save(state)
        systemctl("reset-failed", AGENT)
        systemctl("start", AGENT)
        healthy(state["new_binary_sha256"], timeout)
        systemctl("enable", "--now", *ENABLED)
        state["phase"] = "committed"
        save(state)
        return {"migrated": True, "phase": "committed", "configuration_restored": False,
                "signed_baseline_sha256": state["new_binary_sha256"]}
    except Exception:
        recover()
        raise


def load_state():
    protected(MIGRATION, directory=True)
    protected(MIGRATION / "journal.json")
    state = decode(read(MIGRATION / "journal.json", 16384))
    required = {"schema", "phase", "old_unit_sha256", "old_binary_sha256", "bundle_sha256", "new_binary_sha256"}
    if set(state) != required or state["schema"] != 1:
        raise ValueError("malformed migration state: manual recovery required")
    return state


def recover():
    root_guard()
    if not (MIGRATION / "journal.json").exists():
        return {"recovery": "not_pending"}
    state = load_state()
    if state["phase"] in {"committed", "rolled_back"}:
        return {"recovery": "not_pending", "phase": state["phase"]}
    if state["phase"] not in {"prepared", "stopping", "switching", "trial", "manual_recovery"}:
        raise ValueError("unknown migration phase: manual recovery required")
    old = read(MIGRATION / "old.service", 16384)
    if digest(old) != state["old_unit_sha256"] or digest(read(LEGACY, LIMIT)) != state["old_binary_sha256"]:
        state["phase"] = "manual_recovery"
        save(state)
        raise ValueError("original recovery program/unit changed: manual recovery required")
    try:
        systemctl("disable", "--now", *ENABLED, check=False)
        systemctl("stop", *(UNITS - {AGENT}), check=False)
        current = read(UNIT, 16384)
        if digest(current) != state["old_unit_sha256"]:
            stop_barrier()
            config_barrier()
            atomic(UNIT, old)
        DROPIN.unlink(missing_ok=True)
        if DROPIN.parent.exists():
            sync(DROPIN.parent)
        systemctl("daemon-reload")
        systemctl("reset-failed", AGENT)
        state["phase"] = "rolled_back"
        save(state)
        # Boot recovery is ordered before Agent and cannot synchronously wait on that same job.
        systemctl("start", "--no-block", AGENT)
        return {"recovery": "rolled_back", "configuration_restored": False}
    except Exception:
        state["phase"] = "manual_recovery"
        save(state)
        raise
