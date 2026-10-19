"""Root-only test interposition at the unit publication boundary, never a shipped hook."""

import fcntl
import json
import os
from pathlib import Path
import signal
import subprocess
import sys

from cold_support import wait_for
from enrollment_checks import BEFORE, state, updater
from enrollment_guest import CONFIG, JOURNAL, OPERATOR, UNIT, migrate, migration_args, production, select, setup
from lab_io import atomic_bytes, atomic_json, digest
from setup_guest import guard, unit
from test_guest import systemctl

READY = Path("/etc/nlup-lab/security-gap-ready")


def case(name):
    guard()
    if name != "security_root_gap":
        return boundary_case(name)
    setup()
    migrate("--apply")
    pid, config = production().properties()["MainPID"], CONFIG.read_bytes()
    select("root-gap")
    first = updater("download", success=False)
    second = updater("download", success=False)
    accepted = state()
    assert production().properties()["MainPID"] == pid and CONFIG.read_bytes() == config
    return {"scenario": name, "passed": first.returncode == 0 and second.returncode == 0,
            "first_exit": first.returncode, "second_exit": second.returncode,
            "root_versions": sorted(map(int, accepted["roots"])), "accepted_sequence": accepted["seen_sequence"],
            "old_pid_and_credentials_preserved": True}


def boundary_case(name):
    setup()
    common = production()
    if name in {"security_resume_policy", "security_missing_trust"}:
        atomic_bytes(Path("/var/lib/nlup-enrollment-public/fault"), b"tampered", 0o444)
        assert migrate("--apply", check=False).returncode != 0
        select("base")
        pid, config = common.properties()["MainPID"], CONFIG.read_bytes()
        before = state()
        if name == "security_resume_policy":
            assert migrate("--resume", "--metadata-url", "http://127.0.0.1:18094/other/", check=False).returncode != 0
            assert state() == before
        else:
            path = Path("/var/lib/nodelite-updater/private/journal.json")
            saved = path.with_name("held-journal.fixture")
            path.rename(saved)
            try:
                assert migrate("--resume", check=False).returncode != 0 and not path.exists()
            finally:
                saved.rename(path)
            assert state() == before
        assert common.properties()["MainPID"] == pid and CONFIG.read_bytes() == config
        return {"scenario": name, "passed": True, "pid_credentials_and_trust_preserved": True}
    migrate("--apply")
    before, pid, config = state(), common.properties()["MainPID"], CONFIG.read_bytes()
    if name == "security_updater_lock":
        with open("/var/lib/nodelite-updater/private/lock", "rb") as stream:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            assert updater("download", success=False).returncode != 0
    elif name == "security_root_rejected":
        assert subprocess.run(["/usr/local/bin/nodelite-updater", "download"], capture_output=True).returncode != 0
    elif name == "security_legacy_installer":
        script = Path("/home/lab/agent-update-isolation/installer-guard.fixture.sh")
        result = subprocess.run(["/bin/sh", str(script)], capture_output=True)
        assert result.returncode != 0
    elif name == "security_node_audit":
        result = migrate("--audit")
        assert json.loads(result.stdout)["ready"]
        unit_before = UNIT.read_bytes()
        atomic_bytes(UNIT, unit_before.replace(b"NoNewPrivileges=yes", b"NoNewPrivileges=no"), 0o644)
        try:
            assert migrate("--audit", check=False).returncode != 0
        finally:
            atomic_bytes(UNIT, unit_before, 0o644)
    else:
        raise ValueError("unknown security scenario")
    assert state() == before and common.properties()["MainPID"] == pid and CONFIG.read_bytes() == config
    return {"scenario": name, "passed": True, "pid_credentials_and_trust_preserved": True}


def wrapper():
    guard()
    production()
    from tools.nodelite_migrate import transaction
    from tools.nodelite_migrate.__main__ import main
    original = transaction.atomic

    def pause_after_unit(path, *args, **kwargs):
        original(path, *args, **kwargs)
        if path == UNIT:
            atomic_bytes(READY, b"unit durable, before next statement\n", 0o600)
            os.kill(os.getpid(), signal.SIGSTOP)

    transaction.atomic = pause_after_unit
    sys.argv = [sys.argv[0], *migration_args("--apply")[3:]]
    main()


def arm(name):
    guard()
    if name == "security_committed_boot":
        setup()
        migrate("--apply")
        atomic_json(BEFORE, {"point": name, "config_sha256": digest(CONFIG.read_bytes()),
            "boot": Path("/proc/sys/kernel/random/boot_id").read_text().strip()})
        return {"armed": name, "phase": "committed"}
    if name != "security_switching_backup":
        raise ValueError("unknown security interruption")
    setup()
    READY.unlink(missing_ok=True)
    unit("enrollment.service", f'''[Service]
WorkingDirectory={OPERATOR}
ExecStart=/usr/bin/python3 /home/lab/agent-update-isolation/security_guest.py wrapper
RuntimeMaxSec=120
KillMode=control-group
NoNewPrivileges=yes
''')
    systemctl("daemon-reload")
    os.sync()
    systemctl("start", "nlup-lab-enrollment.service")
    wait_for(lambda: READY.exists(), 90)
    assert json.loads(JOURNAL.read_text())["phase"] == "switching"
    atomic_bytes(JOURNAL.parent / "old.service", b"corrupt fixture backup at unit boundary", 0o600)
    atomic_json(BEFORE, {"point": name, "config_sha256": digest(CONFIG.read_bytes()),
        "boot": Path("/proc/sys/kernel/random/boot_id").read_text().strip()})
    return {"armed": name, "phase": "switching", "recovery_dependency_durable": production().DROPIN.exists()}


def after(name):
    guard()
    before = json.loads(BEFORE.read_text())
    assert before["point"] == name and Path("/proc/sys/kernel/random/boot_id").read_text().strip() != before["boot"]
    if name == "security_committed_boot":
        from enrollment_guest import online
        wait_for(lambda: online("rust-old"))
        assert json.loads(migrate("--audit").stdout)["ready"]
        assert digest(CONFIG.read_bytes()) == before["config_sha256"]
        return {"scenario": name, "passed": True, "committed_not_rolled_back": True, "admission_after_coldboot": True}
    wait_for(lambda: json.loads(JOURNAL.read_text())["phase"] == "manual_recovery", 60)
    process = production().properties()
    assert digest(CONFIG.read_bytes()) == before["config_sha256"]
    return {"scenario": name, "passed": process["MainPID"] == "0", "phase": "manual_recovery",
            "agent_pid": int(process["MainPID"]), "credentials_preserved": True}


if __name__ == "__main__":
    if sys.argv[1:] != ["wrapper"]:
        raise SystemExit("fixed lab wrapper only")
    wrapper()
