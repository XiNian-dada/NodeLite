#!/usr/bin/env python3
"""Root test driver inside the disposable VM; outputs only non-sensitive evidence."""

import grp
import json
import os
from pathlib import Path
import pwd
import secrets
import subprocess
import sys
import time

from lab_io import (BRIDGE, CODE, INBOX, JOURNAL, LIMIT, PRIVATE, ROOT, active_digest,
                    atomic_bytes, atomic_json, decode, digest, object_path, pointer)
from setup_guest import guard

HERE = Path(__file__).resolve().parent
RESULTS = Path("/home/lab/results.json")
ACTIVE_UNITS = ["nlup-lab-request.path", "nlup-lab-reconcile.timer",
                "nlup-lab-stop-bridge.path", "nlup-lab-stop-bridge.service",
                "nlup-lab-preflight-bridge.path", "nlup-lab-restart-bridge.path",
                "nlup-lab-preflight-bridge.service", "nlup-lab-restart-bridge.service",
                "nlup-lab-reconcile.service", "nlup-lab-request.service",
                "nlup-lab-preflight.service", "nlup-lab-agent.service"]


def run(*args, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True, timeout=20)


def systemctl(*args, check=True):
    return run("systemctl", *args, check=check)


def wait(predicate, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.15)
    raise AssertionError("timed out waiting for expected non-sensitive state")


def state():
    return decode(JOURNAL.read_bytes())


def own(path, user="nlup-updater", mode=0o600, group=None):
    os.chown(path, pwd.getpwnam(user).pw_uid, grp.getgrnam(group).gr_gid if group else -1)
    path.chmod(mode)


def healthy(identity):
    try:
        health = decode((ROOT / "status/heartbeat.json").read_bytes())
        pid = systemctl("show", "nlup-lab-agent.service", "--property=MainPID", "--value").stdout.strip()
        return health.get("digest") == identity and str(health.get("pid")) == pid \
            and time.time() - health.get("time", 0) < 2
    except (OSError, ValueError):
        return False


def reset():
    systemctl("stop", *ACTIVE_UNITS, check=False)
    for marker in (Path("/etc/nlup-lab/fault"), PRIVATE / "manual.json", PRIVATE / "rejection.json",
                   PRIVATE / "checkpoint.json",
                   CODE / "quiesce", INBOX / "request.json", BRIDGE / "restart", BRIDGE / "preflight",
                   BRIDGE / "stop"):
        marker.unlink(missing_ok=True)
    baseline = decode((PRIVATE / "baseline.json").read_bytes())
    atomic_json(JOURNAL, baseline)
    own(JOURNAL)
    pointer("active", baseline["current"])
    systemctl("reset-failed", *ACTIVE_UNITS, "nlup-lab-boot.service", check=False)
    systemctl("start", "nlup-lab-agent.service", "nlup-lab-request.path", "nlup-lab-reconcile.timer",
              "nlup-lab-preflight-bridge.path", "nlup-lab-restart-bridge.path", "nlup-lab-stop-bridge.path")
    wait(lambda: healthy(baseline["current"]))
    return baseline


def package(mode="healthy", sequence=2, mutation=None):
    job = secrets.token_hex(16)
    directory = INBOX / job
    directory.mkdir(mode=0o750)
    os.chown(directory, pwd.getpwnam("nlup-agent").pw_uid, pwd.getpwnam("nlup-updater").pw_gid)
    executable = (HERE / "fixture/target/release/nodelite-update-lab-fixture").read_bytes()
    payload = executable + b"\nNLUP_FIXTURE=" + json.dumps({"sequence": sequence, "mode": mode}).encode()
    if mutation == "bad_executable":
        payload = b"not-an-executable"
    manifest = {"schema": 1, "product": "nodelite-lab", "component": "agent", "channel": "stable",
                "target": "x86_64-unknown-linux-gnu", "release_sequence": sequence,
                "metadata_sequence": sequence, "sha256": digest(payload), "size": len(payload),
                "not_after": int(time.time()) + 3600}
    if mutation == "expired":
        manifest["not_after"] = int(time.time()) - 1
    if mutation == "wrong_component":
        manifest["component"] = "server"
    (directory / "agent").write_bytes(payload)
    atomic_json(directory / "manifest.json", manifest)
    run("openssl", "pkeyutl", "-sign", "-inkey", "/root/nlup-lab-signing.pem", "-rawin", "-in",
        str(directory / "manifest.json"), "-out", str(directory / "manifest.sig"))
    for path in directory.iterdir():
        own(path, "nlup-agent", 0o640)
    target = directory / "agent"
    if mutation == "bad_signature":
        (directory / "manifest.sig").write_bytes(bytes(64))
    elif mutation == "bad_digest":
        target.write_bytes(payload + b"tampered")
    elif mutation == "symlink":
        target.unlink()
        target.symlink_to("/etc/passwd")
    elif mutation == "fifo":
        target.unlink()
        os.mkfifo(target, 0o640)
        own(target, "nlup-agent", 0o640)
    elif mutation == "hardlink":
        os.link(target, directory / "extra-link")
    elif mutation == "oversize":
        with target.open("wb") as stream:
            stream.truncate(LIMIT + 1)
    return job, manifest["sha256"]


def request(job):
    atomic_json(INBOX / "request.json", {"job": job})
    own(INBOX / "request.json", "nlup-agent", 0o640)


def permissions():
    forbidden = [CODE / "active", CODE / "objects", PRIVATE / "journal.json", BRIDGE,
                 Path("/etc/systemd/system/nlup-lab-agent.service"), Path("/etc/nlup-lab/release.pub")]
    for path in forbidden:
        result = run("runuser", "-u", "nlup-agent", "--", "test", "-w", str(path), check=False)
        assert result.returncode != 0, f"Agent can write protected path {path.name}"
    result = run("runuser", "-u", "nlup-updater", "--", "test", "-r", str(ROOT / "agent/config"), check=False)
    assert result.returncode != 0, "updater can directly read Agent config"
    for user in ("nlup-agent", "nlup-updater"):
        result = run("runuser", "-u", user, "--", "systemctl", "restart", "nlup-lab-agent.service", check=False)
        assert result.returncode != 0, "ordinary UID acquired system unit management"
    pid = systemctl("show", "nlup-lab-agent.service", "--property=MainPID", "--value").stdout.strip()
    status = Path(f"/proc/{pid}/status").read_text()
    assert "CapEff:\t0000000000000000" in status
    assert "NoNewPrivs:\t1" in status
    assert Path(f"/proc/{pid}").stat().st_uid == pwd.getpwnam("nlup-agent").pw_uid


def sequential():
    committed = None
    for sequence, mode in ((2, "healthy"), (3, "crash"), (4, "crash")):
        job, target = package(mode=mode, sequence=sequence)
        request(job)
        wait(lambda: state().get("job") == job and state()["phase"] in {"committed", "rolled_back"}, timeout=25)
        if sequence == 2:
            committed = target
            assert state()["phase"] == "committed"
        else:
            assert state()["phase"] == "rolled_back" and state()["rollback_consumed"]
            assert state()["current"] == committed
        wait(lambda: healthy(state()["current"]))
    assert state()["metadata_high"] == 4 and len(state()["jobs"]) == 3
    return {"name": "sequential", "passed": True, "transactions": 3}


def invalid_object(name, baseline):
    old_path = object_path(baseline["current"])
    original = old_path.read_bytes()
    try:
        if name == "missing_object":
            old_path.unlink()
        else:
            payload = b"unregistered fixture object"
            target = object_path(digest(payload))
            target.parent.mkdir(mode=0o750, exist_ok=True)
            own(target.parent, mode=0o750, group="nlup-read")
            atomic_bytes(target, payload, 0o550)
            own(target, mode=0o550, group="nlup-read")
            pointer("active", digest(payload))
        wait(lambda: (PRIVATE / "manual.json").exists())
    finally:
        atomic_bytes(old_path, original, 0o550)
        own(old_path, mode=0o550, group="nlup-read")
        pointer("active", baseline["current"])
    return {"name": name, "passed": True, "phase": "manual_recovery"}


def stop_children():
    group = systemctl("show", "nlup-lab-agent.service", "--property=ControlGroup", "--value").stdout.strip()
    assert group.startswith("/system.slice/nlup-lab-agent.service")
    child = subprocess.Popen(["/usr/bin/sleep", "300"])
    try:
        (Path("/sys/fs/cgroup") / group.lstrip("/") / "cgroup.procs").write_text(str(child.pid))
        Path("/etc/nlup-lab/fault").write_text("trial_journal")
        job, _ = package()
        request(job)
        wait(lambda: (PRIVATE / "checkpoint.json").exists())
        assert child.poll() is not None, "old service child survived the switch barrier"
        assert systemctl("show", "nlup-lab-agent.service", "--property=MainPID", "--value").stdout.strip() == "0"
        Path("/etc/nlup-lab/fault").unlink()
        wait(lambda: state()["phase"] == "rolled_back", timeout=25)
    finally:
        if child.poll() is None:
            child.kill()
        child.wait()
    return {"name": "stop_children", "passed": True, "phase": state()["phase"]}


def scenario(name):
    baseline = reset()
    original_config = (ROOT / "agent/config").read_bytes()
    original_pid = systemctl("show", "nlup-lab-agent.service", "--property=MainPID", "--value").stdout.strip()
    if name == "sequential":
        return sequential()
    if name == "stop_children":
        return stop_children()
    if name in {"missing_object", "unexpected_active"}:
        return invalid_object(name, baseline)
    if name == "permissions":
        permissions()
        return {"name": name, "passed": True}
    if name == "manual_recovery":
        atomic_bytes(JOURNAL, b"{")
        own(JOURNAL)
        wait(lambda: (PRIVATE / "manual.json").exists())
        assert healthy(baseline["current"])
        return {"name": name, "passed": True, "phase": "manual_recovery"}
    reject = {"bad_signature", "bad_digest", "symlink", "fifo", "hardlink", "oversize",
              "expired", "wrong_component", "downgrade"}
    mode = "hang" if name == "config_advanced" else name if name in {"crash", "hang", "preflight_fail"} else "healthy"
    job, target = package(mode=mode, sequence=1 if name == "downgrade" else 2,
                          mutation=name if name in reject | {"bad_executable"} else None)
    fault_names = {"kill_prepared": "prepared", "kill_stopping": "stopping", "kill_trial": "trial_journal",
                   "kill_pointer": "active_pointer", "kill_rollback": "rollback_intent"}
    if name in fault_names:
        Path("/etc/nlup-lab/fault").write_text(fault_names[name])
        if name == "kill_rollback":
            job, target = package(mode="crash")
    request(job)
    if name == "concurrent":
        systemctl("start", "--no-block", "nlup-lab-request.service", "nlup-lab-reconcile.service")
    if name in reject:
        wait(lambda: (PRIVATE / "rejection.json").exists())
        assert state()["phase"] == "stable"
        assert systemctl("show", "nlup-lab-agent.service", "--property=MainPID", "--value").stdout.strip() == original_pid
    elif name.startswith("kill_"):
        expected = {"kill_prepared": "prepared", "kill_stopping": "stopping", "kill_trial": "trial", "kill_pointer": "trial",
                    "kill_rollback": "rollback_intent"}[name]
        wait(lambda: (PRIVATE / "checkpoint.json").exists()
             and decode((PRIVATE / "checkpoint.json").read_bytes())["point"] == fault_names[name])
        assert state()["phase"] == expected
        systemctl("stop", "nlup-lab-reconcile.timer", "nlup-lab-request.path")
        wait(lambda: systemctl("is-active", "nlup-lab-request.service", check=False).stdout.strip() != "active")
        Path("/etc/nlup-lab/fault").unlink()
        systemctl("start", "nlup-lab-reconcile.timer")
        wait(lambda: state()["phase"] in {"rolled_back", "committed"}, timeout=25)
    else:
        if name == "config_advanced":
            wait(lambda: state()["phase"] == "trial")
            original_config = secrets.token_bytes(48)
            atomic_bytes(ROOT / "agent/config", original_config)
            own(ROOT / "agent/config", "nlup-agent")
        wait(lambda: state()["phase"] in {"committed", "rolled_back"}, timeout=25)
        expected = "rolled_back" if name in {"crash", "hang", "preflight_fail", "bad_executable", "config_advanced"} else "committed"
        assert state()["phase"] == expected, state()
    assert (ROOT / "agent/config").read_bytes() == original_config, "configuration reverted"
    current = state()["current"]
    assert active_digest() == current
    wait(lambda: healthy(current))
    if state()["phase"] == "rolled_back":
        assert current == baseline["current"]
    if name == "replay":
        generation = state()["generation"]
        request(job)
        wait(lambda: (PRIVATE / "rejection.json").exists())
        assert state()["current"] == target and state()["phase"] == "committed"
        assert state()["generation"] >= generation
    if name == "concurrent":
        assert state()["jobs"] == [job], "duplicate coordinator admitted the same job twice"
    return {"name": name, "passed": True, "phase": state()["phase"],
            "rollback_consumed": state().get("rollback_consumed", False)}


def arm_powercut(point):
    if point not in {"prepared", "stopping", "trial_journal", "active_pointer"}:
        raise ValueError("unknown power-cut checkpoint")
    reset()
    Path("/etc/nlup-lab/fault").write_text(point)
    job, _ = package()
    request(job)
    wait(lambda: (PRIVATE / "checkpoint.json").exists()
         and decode((PRIVATE / "checkpoint.json").read_bytes())["point"] == point)
    systemctl("stop", "nlup-lab-reconcile.timer", "nlup-lab-request.path")
    Path("/etc/nlup-lab/fault").unlink()
    from lab_io import sync_dir
    sync_dir(Path("/etc/nlup-lab"))
    atomic_bytes(Path("/home/lab/power-config-digest"), digest((ROOT / "agent/config").read_bytes()).encode())
    print(json.dumps({"armed": point, "phase": state()["phase"], "active": active_digest()}))


def after_powercut():
    systemctl("start", "nlup-lab-reconcile.timer")
    wait(lambda: state()["phase"] == "rolled_back", timeout=25)
    assert state()["current"] == decode((PRIVATE / "baseline.json").read_bytes())["current"]
    wait(lambda: healthy(state()["current"]))
    assert digest((ROOT / "agent/config").read_bytes()) == Path("/home/lab/power-config-digest").read_text()
    print(json.dumps({"powercut_recovery": "passed", "phase": state()["phase"]}))


def main():
    guard()
    if len(sys.argv) > 1 and sys.argv[1] == "reset-only":
        reset()
        print(json.dumps({"fixture_reset": "passed"}))
        return
    if len(sys.argv) > 1 and sys.argv[1] == "arm":
        arm_powercut(sys.argv[2])
        return
    if len(sys.argv) > 1 and sys.argv[1] == "after-powercut":
        after_powercut()
        return
    if len(sys.argv) > 1 and sys.argv[1] == "arm-corrupt":
        reset()
        atomic_bytes(JOURNAL, b"{")
        own(JOURNAL)
        print(json.dumps({"armed": "corrupt_journal"}))
        return
    if len(sys.argv) > 1 and sys.argv[1] == "after-corrupt":
        wait(lambda: (PRIVATE / "manual.json").exists())
        assert systemctl("is-active", "nlup-lab-agent.service", check=False).returncode != 0
        print(json.dumps({"corrupt_boot_gate": "passed"}))
        return
    names = sys.argv[1:] or ["permissions", "success", "preflight_fail", "bad_executable", "crash", "hang",
                             "bad_signature", "bad_digest", "symlink", "fifo", "hardlink", "oversize",
                             "expired", "wrong_component", "downgrade", "replay", "config_advanced",
                             "kill_prepared", "kill_stopping", "stop_children", "kill_trial", "kill_pointer", "kill_rollback",
                             "concurrent", "sequential", "missing_object", "unexpected_active", "manual_recovery"]
    results = []
    for name in names:
        result = scenario(name)
        results.append(result)
        atomic_json(RESULTS, results)
        print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
