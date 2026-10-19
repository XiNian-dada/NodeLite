"""Adversarial and cold-boot checks for the real Rust updater and real Agent credentials."""

import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time

from cold_support import api, wait_for
from lab_io import atomic_bytes, atomic_json
from rust_guest import AGENT, BINARIES, HOME, POLICY, PUBLIC, enroll, health, state, updater
from setup_guest import directory, run
from test_guest import systemctl

POINTS = ("root_persisted", "cache_committed", "artifact_durable", "prepared", "stopping", "trial_journal", "active_pointer")
NOTICE = Path("/run/nodelite-updater-lab/checkpoint.json")


def select(name):
    if name not in {"good", "tampered", "expired", "wrong_scope", "one_signer", "downgrade", "equivocation",
                    "latest", "crash", "hang", "preflight_fail", "revoked", "revoked_without_root", "rotated_good", "root_one_old", "root_one_new"}:
        raise ValueError("unknown signed lab repository")
    temporary = PUBLIC / ".next"
    temporary.symlink_to("variants/" + name)
    os.replace(temporary, PUBLIC / "current")


def pid():
    return int(run("systemctl", "show", "nodelite-agent.service", "--property=MainPID", "--value").stdout)


def rotated():
    previous = (AGENT / "agent.toml").read_bytes()
    assert api("/api/nodes/cold-node/refresh-token", {})["ok"]
    latest = (AGENT / "agent.toml").read_bytes()
    assert latest != previous
    return latest


def assert_old(latest, original_pid=None):
    wait_for(lambda: health("rust-old"))
    assert (AGENT / "agent.toml").read_bytes() == latest
    if original_pid is not None:
        assert pid() == original_pid


def permissions():
    def denied(user, *command):
        return subprocess.run(["runuser", "-u", user, "--", *command], capture_output=True).returncode != 0
    assert denied("nodelite-updater", "test", "-r", str(AGENT / "agent.toml"))
    for path in (HOME / "code", HOME / "private", HOME / "bridge"):
        assert denied("nodelite-agent", "test", "-w", str(path))
    assert denied("nodelite-updater", "sudo", "-n", "true")
    assert "security.capability" not in os.listxattr(HOME / "code/active")
    return {"direct_token_read_denied": True, "agent_code_state_bridge_writes_denied": True,
            "updater_sudo_denied": True, "file_capabilities_absent": True}


def case(name):
    enroll()
    latest, original_pid = rotated(), pid()
    evidence = {"scenario": name, "passed": True}
    if name == "permissions":
        evidence.update(permissions())
    elif name in {"tampered", "expired", "wrong_scope", "one_signer", "root_503", "slow_timestamp", "root_one_old", "root_one_new"}:
        if name in {"root_503", "slow_timestamp"}:
            atomic_bytes(PUBLIC / "fault", name.encode(), 0o444)
        else:
            select(name)
        started = time.monotonic()
        result = updater("apply", success=False)
        assert result.returncode != 0
        duration = time.monotonic() - started
        assert duration < 15
        assert state()["phase"] == "stable" and state()["installed"]["sequence"] == 1
        assert_old(latest, original_pid)
        evidence["elapsed_secs"] = round(duration, 3)
    elif name in {"crash", "hang", "preflight_fail", "trial_token_rollback"}:
        select("good" if name == "trial_token_rollback" else name)
        updater("apply")
        if name == "trial_token_rollback":
            wait_for(lambda: state()["phase"] == "trial" and health("rust-new"))
            latest = rotated()
            os.chown("/run/nodelite-agent", 0, 0)
            os.chmod("/run/nodelite-agent", 0o555)
        wait_for(lambda: state()["phase"] == "rolled_back", 65)
        assert_old(latest, original_pid if name == "preflight_fail" else None)
        assert state()["installed"]["sequence"] == 1 and state()["seen_sequence"] == 2
        assert updater("apply", success=False).returncode != 0, "failed target was automatically retried"
        evidence.update(latest_credentials_preserved=True, rollback_consumed=state()["transaction"]["rollback_consumed"])
    elif name == "failed_download_downgrade":
        select("tampered")
        assert updater("download", success=False).returncode != 0
        assert state()["seen_sequence"] == 2
        select("downgrade")
        assert updater("apply", success=False).returncode != 0
        assert_old(latest, original_pid)
        select("latest")
        updater("apply")
        wait_for(lambda: state()["phase"] == "committed", 50)
        evidence["accepted_floor_survived_failure"] = True
    elif name == "root_revocation":
        select("revoked")
        assert updater("download", success=False).returncode != 0
        root = HOME / "private/generations" / state()["cache"] / "root.json"
        assert json.loads(root.read_text())["signed"]["version"] == 2
        select("revoked_without_root")
        assert updater("download", success=False).returncode != 0
        assert_old(latest, original_pid)
        select("rotated_good")
        updater("apply")
        wait_for(lambda: state()["phase"] == "committed", 50)
        evidence["revoked_keys_remained_rejected"] = True
    elif name == "corrupt_journal":
        atomic_bytes(HOME / "private/journal.json", b"{corrupt")
        assert updater("reconcile", success=False).returncode != 0
        assert (HOME / "private/recovery-required.json").exists()
        assert_old(latest, original_pid)
    else:
        raise ValueError("unknown Rust fault scenario")
    return evidence


def arm(point):
    if point not in POINTS:
        raise ValueError("unknown power-cut boundary")
    enroll()
    latest = rotated()
    if point == "root_persisted":
        select("rotated_good")
    shutil.copyfile(BINARIES / "nodelite-updater-faults", "/usr/local/bin/nodelite-updater")
    Path("/usr/local/bin/nodelite-updater").chmod(0o555)
    directory(NOTICE.parent, "nodelite-updater", "nodelite-updater", 0o700)
    NOTICE.unlink(missing_ok=True)
    for name in ("apply", "reconcile", "boot"):
        unit_directory = Path("/etc/systemd/system") / f"nodelite-updater-{name}.service.d"
        unit_directory.mkdir(exist_ok=True)
        atomic_bytes(unit_directory / "lab-fault.conf", b"[Service]\nReadWritePaths=-/run/nodelite-updater-lab\n", 0o644)
    systemctl("daemon-reload")
    atomic_bytes(POLICY / "fault", point.encode(), 0o644)
    atomic_json(POLICY / "before.json", {"config_sha256": __import__("hashlib").sha256(latest).hexdigest(),
                "boot": Path("/proc/sys/kernel/random/boot_id").read_text().strip()}, )
    os.sync()
    systemctl("start", "--no-block", "nodelite-updater-apply.service")
    wait_for(NOTICE.exists, 30)
    evidence = json.loads(NOTICE.read_text())
    assert evidence["point"] == point
    return {"armed": point, "phase": state()["phase"]}


def after(point):
    before = json.loads((POLICY / "before.json").read_text())
    assert Path("/proc/sys/kernel/random/boot_id").read_text().strip() != before["boot"]
    (POLICY / "fault").unlink()
    expected = "stable" if point in {"root_persisted", "cache_committed", "artifact_durable"} else "rolled_back"
    wait_for(lambda: state()["phase"] == expected, 65)
    wait_for(lambda: health("rust-old"))
    assert __import__("hashlib").sha256((AGENT / "agent.toml").read_bytes()).hexdigest() == before["config_sha256"]
    if point == "root_persisted":
        assert "2" in state()["roots"]
        select("revoked_without_root")
        assert updater("download", success=False).returncode != 0
        select("rotated_good")
    else:
        assert state()["seen_sequence"] == 2
    if expected == "stable":
        updater("apply")
        wait_for(lambda: state()["phase"] == "committed", 50)
    return {"point": point, "passed": True, "recovery_phase": expected, "new_boot": True,
            "credentials_preserved": True, "accepted_sequence": state()["seen_sequence"]}
