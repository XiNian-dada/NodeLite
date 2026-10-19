"""Real Rust updater ENOSPC on bounded noexec loop volumes, never the guest root disk."""

import os
from pathlib import Path
import secrets

from resilience_volume import exhaust, release, volume
from rust_guest import HOME, UNITS, enroll, updater
from rust_checks import assert_old, pid, rotated
from setup_guest import run
from test_guest import systemctl


def case(name):
    kind = "inode" if name == "storage_inode" else "disk"
    partial = name == "storage_target"
    enroll()
    latest, original_pid = rotated(), pid()
    systemctl("stop", "nodelite-updater-reconcile.timer", "nodelite-updater-reconcile.service")
    with volume() as mountpoint:
        copy = mountpoint / "home"
        copy.mkdir()
        run("cp", "-a", str(HOME) + "/.", str(copy))
        run("mount", "--bind", str(copy), str(HOME))
        try:
            fault, fillers = exhaust(mountpoint, kind, 512 * 1024 if partial else 0)
            rejected = updater("download", success=False)
            assert rejected.returncode != 0 and "os error 28" in rejected.stderr
            assert_old(latest, original_pid)
            release(fillers)
            updater("download")
            import json
            state = json.loads((HOME / "private/journal.json").read_text())
            assert state["phase"] == "stable" and state["installed"]["sequence"] == 1 and state["seen_sequence"] == 2
            assert_old(latest, original_pid)
            result = {"scenario": name, "passed": True, "fault": fault, "old_process_preserved": True,
                      "credentials_preserved": True, "retry_download": True}
        finally:
            systemctl("stop", *UNITS, check=False)
            # Keep the latest trust journal after removing the bind mount, just as with tokens.
            saved = HOME.with_name("nodelite-updater.saved." + secrets.token_hex(8))
            saved.mkdir(mode=0o755)
            run("cp", "-a", str(HOME) + "/.", str(saved))
            os.sync()
            assert os.stat(HOME).st_dev == os.stat(mountpoint).st_dev
            run("umount", str(HOME))
            HOME.rename(HOME.with_name("nodelite-updater.archived." + secrets.token_hex(8)))
            saved.rename(HOME)
            os.sync()
    return result
