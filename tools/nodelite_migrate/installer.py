"""Installer-only orchestration; trust and rollback remain in the existing migration/updater."""

import time

from tools.nodelite_release.io import decode, read

from . import bundle, transaction
from .common import MIGRATION, UPDATER, command


def validate(path, target):
    descriptor = decode(read(path / "enrollment.json", 16384))
    if set(descriptor) != {"schema", "bundle_sha256", "root_sha256", "metadata_url", "targets_url"} \
            or descriptor["schema"] != 1:
        raise ValueError("invalid enrollment descriptor")
    manifest, files = bundle.inspect(path, descriptor["bundle_sha256"], descriptor["root_sha256"])
    if manifest["target"] != target:
        raise ValueError("enrollment architecture mismatch")
    bundle.policy(manifest, files, descriptor["metadata_url"], descriptor["targets_url"])
    return (path, descriptor["bundle_sha256"], descriptor["root_sha256"],
            descriptor["metadata_url"], descriptor["targets_url"])


def enroll(path, target):
    args = validate(path, target)
    if (MIGRATION / "journal.json").exists():
        transaction.recover()
        return transaction.resume(*args)
    return transaction.apply(*args)


def status():
    result = command("/usr/sbin/runuser", "-u", "nodelite-updater", "-g", "nodelite-read", "-G",
                     "nodelite-updater", "--", UPDATER, "status", check=False)
    if result.returncode:
        return None  # The reconcile timer briefly owns the updater lock.
    return decode(result.stdout)


def outcome(state):
    if state is None:
        return False
    if state["manual_recovery"] or state["phase"] in {"rolled_back", "manual_recovery"}:
        raise RuntimeError("signed update failed; previous Agent preserved, inspect local updater logs")
    if state["phase"] not in {"stable", "committed", "prepared", "stopping", "trial", "rollback_intent"}:
        raise ValueError("unknown updater phase")
    return state["phase"] in {"stable", "committed"}


def upgrade(timeout=180):
    from .audit import check

    deadline = time.monotonic() + timeout
    initial = status()
    while initial is None and time.monotonic() < deadline:
        time.sleep(1)
        initial = status()
    if initial is None:
        raise RuntimeError("updater remained busy; retry the same command")
    if initial is not None and initial["phase"] in {"stable", "committed", "rolled_back"}:
        check()
        command("/usr/bin/systemctl", "start", "nodelite-updater-apply.service", timeout=100)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        state = status()
        if outcome(state):
            check()
            return {"updated": True, "phase": state["phase"], "installed_sequence": state["installed_sequence"]}
        time.sleep(1)
    raise RuntimeError("update still pending; safe reconciliation continues, rerun the same command to check")
