"""Adversarial admin inputs and real KVM interruption checkpoints for first migration."""

import json
import os
from pathlib import Path
import re
import secrets
import shlex
import subprocess
import time

from cold_support import api, wait_for
from enrollment_guest import (CONFIG, INPUT, JOURNAL, OPERATOR, PUBLIC, UNIT, migration_args, migrate,
                              online, production, rotate, select, setup)
from lab_io import atomic_bytes, atomic_json, digest
from setup_guest import run, unit
from test_guest import systemctl

BEFORE = Path("/etc/nlup-lab/enrollment-before.json")


def updater(action, success=True):
    result = subprocess.run(["/usr/sbin/runuser", "-u", "nodelite-updater", "-g", "nodelite-read", "-G", "nodelite-updater", "--",
                             "/usr/local/bin/nodelite-updater", action], capture_output=True, text=True, timeout=90)
    if success and result.returncode:
        raise RuntimeError(result.stderr[-1000:])
    return result


def state():
    return json.loads(Path("/var/lib/nodelite-updater/private/journal.json").read_text())


def case(name):
    common = production()
    if name in {"timestamp_refresh", "root_rotation", "revoked_release_key"}:
        return signing_case(name)
    if name == "outdated_bundle_resume":
        return outdated_bundle()
    if name == "live_enrollment_rejected":
        return live_enrollment()
    setup()
    pid, config = common.properties()["MainPID"], CONFIG.read_bytes()
    flags = []
    if name == "bad_bundle_pin":
        flags = ["--bundle-sha256", secrets.token_hex(32)]
    elif name == "bad_root_pin":
        flags = ["--root-sha256", secrets.token_hex(32)]
    elif name == "traffic_control":
        atomic_bytes(UNIT, UNIT.read_bytes().replace(b"AmbientCapabilities=\n", b"AmbientCapabilities=CAP_NET_ADMIN\n"), 0o644)
        systemctl("daemon-reload")
    elif name == "stale_credentials":
        bad = re.sub(rb'(?m)^token = ".*"$', b'token = "' + secrets.token_urlsafe(32).encode() + b'"', config, count=1)
        assert bad != config
        atomic_bytes(CONFIG, bad, 0o600)
        import pwd
        user = pwd.getpwnam("nodelite-agent")
        os.chown(CONFIG, user.pw_uid, user.pw_gid)
        config = bad
    elif name == "tampered_download":
        atomic_bytes(PUBLIC / "fault", b"tampered", 0o444)
    elif name == "trial_token_rollback":
        return trial_failure()
    else:
        raise ValueError("unknown migration failure test")
    assert migrate("--apply", *flags, check=False).returncode != 0
    wait_for(lambda: online("3.0.4"))
    assert common.properties()["MainPID"] == pid and CONFIG.read_bytes() == config
    if JOURNAL.exists():
        assert json.loads(JOURNAL.read_text())["phase"] == "rolled_back"
    if name in {"stale_credentials", "tampered_download"}:
        previous = state()
        if name == "stale_credentials":
            config = rotate()
        else:
            select("base")
        migrate("--resume")
        wait_for(lambda: online("rust-old"))
        assert CONFIG.read_bytes() == config and state()["seen_sequence"] >= previous["seen_sequence"]
        return {"scenario": name, "passed": True, "rejected_before_stop": True,
                "resume_preserved_trust_and_credentials": True}
    return {"scenario": name, "passed": True, "old_pid_preserved": True, "config_bytes_preserved": True}


def trial_failure():
    process = subprocess.Popen(migration_args("--apply", "--health-timeout", "12"), cwd=OPERATOR,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    wait_for(lambda: JOURNAL.exists() and json.loads(JOURNAL.read_text())["phase"] == "trial" and online("rust-old"), 60)
    freeze = Path("/run/nodelite-agent")
    os.chown(freeze, 0, 0)
    freeze.chmod(0o555)
    latest = rotate()
    process.communicate(timeout=60)
    assert process.returncode != 0
    wait_for(lambda: online("3.0.4"))
    assert CONFIG.read_bytes() == latest and json.loads(JOURNAL.read_text())["phase"] == "rolled_back"
    return {"scenario": "trial_token_rollback", "passed": True, "latest_credentials_preserved": True,
            "legacy_authenticated_after_rollback": True}


def signing_case(name):
    common = production()
    pid, config = common.properties()["MainPID"], CONFIG.read_bytes()
    if name == "timestamp_refresh":
        select("refresh")
        updater("download")
        assert common.properties()["MainPID"] == pid and state()["installed"]["sequence"] == 1
        cache = Path("/var/lib/nodelite-updater/private/generations") / state()["cache"]
        assert json.loads((cache / "timestamp.json").read_text())["signed"]["version"] == 2
        assert json.loads((cache / "targets.json").read_text())["signed"]["version"] == 1
    elif name == "root_rotation":
        select("rotated")
        updater("apply")
        wait_for(lambda: state()["phase"] == "committed" and state()["installed"]["sequence"] == 2, 65)
        wait_for(lambda: online("rust-new"))
        assert "2" in state()["roots"]
    else:
        select("revoked")
        assert updater("download", success=False).returncode != 0
        assert common.properties()["MainPID"] == pid and state()["installed"]["sequence"] == 2
        select("rotated-recovery")
        updater("download")
        assert common.properties()["MainPID"] == pid
    assert CONFIG.read_bytes() == config
    return {"scenario": name, "passed": True, "root_history_retained": True, "config_bytes_preserved": True}


def outdated_bundle():
    setup()
    common = production()
    pid, config = common.properties()["MainPID"], CONFIG.read_bytes()
    select("rotated")
    assert migrate("--apply", check=False).returncode != 0
    previous = state()
    assert previous["seen_sequence"] == 2 and "2" in previous["roots"]
    assert common.properties()["MainPID"] == pid and CONFIG.read_bytes() == config
    info = json.loads((INPUT / "fingerprints.json").read_text())["next"]
    migrate("--resume", "--bundle", str(INPUT / "bundle-next"), "--bundle-sha256", info["bundle_sha256"],
            "--root-sha256", info["root_sha256"])
    wait_for(lambda: online("rust-new"))
    assert CONFIG.read_bytes() == config and state()["seen_sequence"] == 2
    assert state()["roots"] == previous["roots"] and json.loads(JOURNAL.read_text())["phase"] == "committed"
    return {"scenario": "outdated_bundle_resume", "passed": True, "old_process_kept_until_authorized": True,
            "root_history_and_sequence_floor_retained": True}


def live_enrollment():
    setup()
    migrate("--apply")
    pid, before = production().properties()["MainPID"], state()
    result = subprocess.run(["/usr/sbin/runuser", "-u", "nodelite-updater", "-g", "nodelite-read", "-G", "nodelite-updater", "--",
        "/usr/local/bin/nodelite-updater", "verify-enrollment", "--baseline", "/usr/local/lib/nodelite-enrollment-agent"],
        capture_output=True, timeout=90)
    assert result.returncode != 0 and state() == before and production().properties()["MainPID"] == pid
    return {"scenario": "live_enrollment_rejected", "passed": True, "active_pointers_and_pid_unchanged": True}


def arm(point):
    if point not in {"prepared", "trial", "trial_token", "corrupt_backup"}:
        raise ValueError("unknown migration checkpoint")
    setup()
    if point == "prepared":
        atomic_bytes(PUBLIC / "fault", b"slow", 0o444)
    args = " ".join(shlex.quote(str(arg)) for arg in migration_args("--apply", "--health-timeout", "30"))
    unit("enrollment.service", f'''[Service]
WorkingDirectory={OPERATOR}
ExecStart={args}
RuntimeMaxSec=120
KillMode=control-group
NoNewPrivileges=yes
''')
    systemctl("daemon-reload")
    os.sync()
    systemctl("start", "nlup-lab-enrollment.service")
    expected = "prepared" if point == "prepared" else "trial"
    wait_for(lambda: JOURNAL.exists() and json.loads(JOURNAL.read_text())["phase"] == expected, 65)
    if point in {"trial_token", "corrupt_backup"}:
        wait_for(lambda: online("rust-old"))
        freeze = Path("/run/nodelite-agent")
        os.chown(freeze, 0, 0)
        freeze.chmod(0o555)
        rotate()
    if point == "corrupt_backup":
        atomic_bytes(JOURNAL.parent / "old.service", b"corrupt fixture backup", 0o600)
    atomic_json(BEFORE, {"config_sha256": digest(CONFIG.read_bytes()),
        "boot": Path("/proc/sys/kernel/random/boot_id").read_text().strip(), "point": point})
    return {"armed": point, "phase": json.loads(JOURNAL.read_text())["phase"]}


def after(point):
    before = json.loads(BEFORE.read_text())
    assert before["point"] == point and Path("/proc/sys/kernel/random/boot_id").read_text().strip() != before["boot"]
    if point == "corrupt_backup":
        wait_for(lambda: production().properties()["MainPID"] == "0")
        result = systemctl("show", "nodelite-updater-migration-recover.service", "--property=Result", "--value").stdout.strip()
        assert result != "success"
    else:
        wait_for(lambda: json.loads(JOURNAL.read_text())["phase"] == "rolled_back", 65)
        wait_for(lambda: online("3.0.4"))
        time.sleep(2)
        assert online("3.0.4")
    assert digest(CONFIG.read_bytes()) == before["config_sha256"]
    return {"scenario": "powercut_" + point, "passed": True, "new_boot": True,
            "latest_credentials_preserved": True, "manual_recovery" if point == "corrupt_backup" else "legacy_recovered": True}
