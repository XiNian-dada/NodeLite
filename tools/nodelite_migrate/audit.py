"""Post-migration admission check without restarting a service or reading Agent credentials."""

import json
from pathlib import Path
import pwd
import stat
import time

from tools.nodelite_release.io import LIMIT, decode, digest, read

from .common import (AGENT, CONFIG, DROPIN, ENABLED, MIGRATION, POLICY, RECOVERY, UNIT, UNITS,
                     UPDATER, command, properties, protected, root_guard, systemctl)
from .transaction import load_state, validate_accounts


def health_matches(evidence, process, expected, wall, boot):
    try:
        return evidence["schema"] == 1 and evidence["pid"] == int(process["MainPID"]) \
            and evidence["invocation"] == process["InvocationID"] and evidence["executable_sha256"] == expected \
            and evidence["sample_counter"] > 0 and 0 <= wall - evidence["collected_at_ms"] < 15000 \
            and 0 <= boot - evidence["sample_boottime_ms"] < 15000
    except (ValueError, KeyError, TypeError):
        return False


def check():
    root_guard()
    if load_state()["phase"] != "committed":
        raise ValueError("migration is not committed")
    protected(MIGRATION / "receipt.json")
    manifest = decode(read(MIGRATION / "receipt.json", 16384))["manifest"]["files"]
    for name, path in (("root.json", POLICY / "root.json"), ("nodelite-updater", UPDATER)):
        protected(path)
        if digest(read(path, LIMIT)) != manifest[name]:
            raise ValueError("root trust or updater program changed")
    for name in UNITS:
        path = UNIT.parent / name
        protected(path)
        if digest(read(path, 16384)) != manifest["systemd/" + name]:
            raise ValueError("fixed unit differs from the independently approved bundle")
    protected(DROPIN)
    if read(DROPIN) != f"[Unit]\nRequires={RECOVERY}\nAfter={RECOVERY}\n".encode():
        raise ValueError("migration recovery dependency changed")
    process = properties()
    if process["DropInPaths"] != str(DROPIN) or process["FragmentPath"] != str(UNIT) \
            or process["User"] != "nodelite-agent" or process["Group"] != "nodelite-read" \
            or process["AmbientCapabilities"] or process["CapabilityBoundingSet"] or process["ActiveState"] != "active":
        raise ValueError("unexpected or privileged Agent installation")
    for name in (*ENABLED, AGENT, RECOVERY):
        systemctl("is-enabled", name)
        systemctl("is-active", name)
    agent = pwd.getpwnam("nodelite-agent")
    for path, mode in ((CONFIG.parent, 0o700), (CONFIG, 0o600)):
        info = path.lstat()
        valid_type = stat.S_ISDIR(info.st_mode) if path == CONFIG.parent else stat.S_ISREG(info.st_mode) and info.st_nlink == 1
        if not valid_type or info.st_uid != agent.pw_uid or stat.S_IMODE(info.st_mode) != mode:
            raise ValueError("unsafe Agent configuration ownership or inode")
    validate_accounts()
    output = command("/usr/sbin/runuser", "-u", "nodelite-updater", "-g", "nodelite-read", "-G", "nodelite-updater", "--", UPDATER, "status")
    status = json.loads(output.stdout)
    if status["manual_recovery"] or status["phase"] not in {"stable", "committed", "rolled_back"}:
        raise ValueError("updater is not in an admitted stable state")
    evidence = decode(read(Path("/run/nodelite-agent/update-health.json"), 4096))
    if not health_matches(evidence, process, status["installed_sha256"], time.time() * 1000,
                          time.clock_gettime(time.CLOCK_BOOTTIME) * 1000):
        raise ValueError("authenticated Agent health is not fresh")
    with open(f"/proc/{int(process['MainPID'])}/exe", "rb") as stream:
        if digest(stream.read(LIMIT + 1)) != status["installed_sha256"]:
            raise ValueError("actual process image differs from the admitted release")
    return {"ready": True, "migration": "committed", "installed_sequence": status["installed_sequence"],
            "installed_sha256": status["installed_sha256"], "credentials_read": False,
            "fixed_units_verified": len(UNITS), "fresh_process_health": True}
