#!/usr/bin/env python3
"""Experimental non-root coordinator; never install on a production host."""

import fcntl
import os
from pathlib import Path
import pwd
import re
import signal
import subprocess
import sys
import time

from lab_io import (BRIDGE, CODE, INBOX, JOURNAL, LIMIT, PRIVATE, ROOT, active_digest,
                    atomic_bytes, atomic_json, bounded_at, decode, digest, object_path,
                    pointer, read_file, sync_dir)

TERMINAL = {"stable", "committed", "rolled_back"}
PHASES = TERMINAL | {"prepared", "stopping", "trial", "rollback_intent", "manual_recovery"}


def properties(unit):
    result = subprocess.run(["/usr/bin/systemctl", "show", unit, "--property=MainPID,InvocationID,"
                             "ActiveState,ControlGroup,Result,ExecMainStatus,NRestarts,ExecMainExitTimestampMonotonic"], capture_output=True,
                            text=True, check=True, timeout=3)
    return dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)


def boot_id():
    return Path("/proc/sys/kernel/random/boot_id").read_text().strip()


def save(state, phase=None, result=None):
    if phase:
        state["phase"] = phase
    if result:
        state["result"] = result
    state["generation"] += 1
    atomic_json(JOURNAL, state)


def manual(reason):
    atomic_json(PRIVATE / "manual.json", {"result": reason})


def fault(name):
    path = Path("/etc/nlup-lab/fault")
    if path.exists() and path.read_text().strip() == name:
        atomic_json(PRIVATE / "checkpoint.json", {"point": name})
        os.kill(os.getpid(), signal.SIGKILL)


def marker(name):
    atomic_bytes(BRIDGE / name, b"request\n", 0o600)


def clear_bridge_requests():
    for name in ("stop", "restart", "preflight"):
        (BRIDGE / name).unlink(missing_ok=True)
    sync_dir(BRIDGE)


def verify_packet(job):
    directory = os.open(INBOX / job, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        manifest = bounded_at(directory, "manifest.json", 4096)
        signature = bounded_at(directory, "manifest.sig", 64)
        payload = bounded_at(directory, "agent", LIMIT)
    finally:
        os.close(directory)
    atomic_bytes(PRIVATE / "manifest", manifest)
    atomic_bytes(PRIVATE / "signature", signature)
    subprocess.run(["/usr/bin/openssl", "pkeyutl", "-verify", "-pubin", "-inkey",
                    "/etc/nlup-lab/release.pub", "-rawin", "-in", str(PRIVATE / "manifest"),
                    "-sigfile", str(PRIVATE / "signature")], check=True, timeout=3,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    item = decode(manifest)
    expected = {"schema", "product", "component", "target", "channel", "release_sequence",
                "metadata_sequence", "sha256", "size", "not_after"}
    if set(item) != expected or item["schema"] != 1 or item["product"] != "nodelite-lab" \
            or item["component"] != "agent" or item["target"] != "x86_64-unknown-linux-gnu" \
            or item["channel"] != "stable":
        raise ValueError("invalid release scope")
    for key in ("release_sequence", "metadata_sequence", "size", "not_after"):
        if type(item[key]) is not int or not 0 < item[key] < 2**53:
            raise ValueError("invalid integer")
    if item["not_after"] < time.time() or item["not_after"] > time.time() + 86400:
        raise ValueError("invalid expiry")
    if item["sha256"] != digest(payload) or item["size"] != len(payload):
        raise ValueError("payload mismatch")
    item["manifest_digest"] = digest(manifest)
    return item, payload


def prepare(state, job):
    if not re.fullmatch(r"[0-9a-f]{32}", job) or job in state["jobs"]:
        raise ValueError("invalid or replayed job")
    release, payload = verify_packet(job)
    policy = decode(read_file(Path("/etc/nlup-lab/policy.json")))
    if release["metadata_sequence"] <= state["metadata_high"] \
            or release["release_sequence"] <= state["sequence"] \
            or state["sequence"] < policy["minimum_sequence"] \
            or release["sha256"] in state["failed"]:
        raise ValueError("release policy rejected")
    if active_digest() != state["current"]:
        raise ValueError("baseline mismatch")
    destination = object_path(release["sha256"])
    destination.parent.mkdir(mode=0o750, exist_ok=True)
    destination.parent.chmod(0o750)
    atomic_bytes(destination, payload, 0o550)
    sync_dir(destination.parent.parent)
    pointer("candidate", release["sha256"])
    old = properties("nlup-lab-agent.service")
    probe = properties("nlup-lab-preflight.service")
    state.update({"job": job, "from": state["current"], "to": release["sha256"],
                  "to_sequence": release["release_sequence"], "manifest": release["manifest_digest"],
                  "metadata_high": release["metadata_sequence"], "rollback_consumed": False,
                  "recovery_requested": False, "stable_invocation": None,
                  "stable_sample": None,
                  "boot_id": boot_id(), "deadline": time.monotonic() + 8,
                  "old_invocation": old.get("InvocationID"),
                  "probe_exit": probe.get("ExecMainExitTimestampMonotonic", "0"),
                  "stable_since": None, "jobs": (state["jobs"] + [job])[-64:]})
    save(state, "prepared", "prepared")
    fault("prepared")
    marker("preflight")


def heartbeat():
    try:
        return decode(read_file(ROOT / "status/heartbeat.json"))
    except (OSError, ValueError):
        return {}


def rollback(state, reason):
    policy = decode(read_file(Path("/etc/nlup-lab/policy.json")))
    if state["sequence"] < policy["minimum_sequence"]:
        manual("baseline revoked")
        return
    if state["phase"] != "rollback_intent":
        state["rollback_consumed"] = True
        state["failed"] = (state["failed"] + [state["to"]])[-64:]
        save(state, "rollback_intent", reason)
        fault("rollback_intent")
    if digest(read_file(object_path(state["from"]), LIMIT)) != state["from"]:
        manual("rollback object mismatch")
        return
    health = heartbeat()
    current = properties("nlup-lab-agent.service")
    if active_digest() == state["from"] and current.get("ActiveState") == "active" \
            and health.get("digest") == state["from"] \
            and str(health.get("pid")) == current.get("MainPID") \
            and time.time() - health.get("time", 0) < 2:
        (CODE / "quiesce").unlink(missing_ok=True)
        state["current"] = state["from"]
        save(state, "rolled_back", reason)
        return
    if not state.get("recovery_requested"):
        pointer("active", state["from"])
        fault("rollback_pointer")
        (CODE / "quiesce").unlink(missing_ok=True)
        marker("restart")
        state["recovery_requested"] = True
        state["recovery_boot"] = boot_id()
        state["recovery_deadline"] = time.monotonic() + 10
        save(state)
    elif state["recovery_boot"] != boot_id():
        state["recovery_boot"] = boot_id()
        state["recovery_deadline"] = time.monotonic() + 10
        save(state)
    elif time.monotonic() > state["recovery_deadline"]:
        manual("rollback service did not recover")


def prepared(state):
    if state["boot_id"] != boot_id():
        if active_digest() != state["from"]:
            manual("prepared pointer mismatch")
            return
        (CODE / "quiesce").unlink(missing_ok=True)
        save(state, "rolled_back", "prepared cancelled after reboot")
        return
    probe = properties("nlup-lab-preflight.service")
    done = probe.get("ExecMainExitTimestampMonotonic", "0") not in {"0", state["probe_exit"]} \
        and probe.get("ActiveState") != "activating"
    if done and (probe.get("Result") != "success" or probe.get("ExecMainStatus") != "0"):
        state["failed"] = (state["failed"] + [state["to"]])[-64:]
        save(state, "rolled_back", "preflight rejected")
        return
    if done:
        state["deadline"] = time.monotonic() + 10
        state["stop_exit"] = properties("nlup-lab-stop-bridge.service").get("ExecMainExitTimestampMonotonic", "0")
        save(state, "stopping", "waiting for old process exit")
        fault("stopping")
        marker("stop")
        return
    if time.monotonic() > state["deadline"]:
        (CODE / "quiesce").unlink(missing_ok=True)
        save(state, "rolled_back", "preflight timeout")


def stopping(state):
    if state["boot_id"] != boot_id():
        if active_digest() != state["from"]:
            manual("stopping pointer mismatch")
            return
        save(state, "rolled_back", "stopping cancelled after reboot")
        return
    current = properties("nlup-lab-agent.service")
    stopped = properties("nlup-lab-stop-bridge.service")
    acknowledged = stopped.get("ExecMainExitTimestampMonotonic", "0") not in {"0", state.get("stop_exit", "0")} \
        and stopped.get("ActiveState") == "inactive" and stopped.get("Result") == "success"
    group = current.get("ControlGroup", "")
    processes = Path("/sys/fs/cgroup") / group.lstrip("/") / "cgroup.procs"
    empty = not group or not processes.exists() or not processes.read_text().strip()
    if acknowledged and not (BRIDGE / "stop").exists() \
            and current.get("ActiveState") == "inactive" and current.get("MainPID") == "0" and empty:
        state["deadline"] = time.monotonic() + 10
        save(state, "trial", "old service stopped")
        fault("trial_journal")
        pointer("active", state["to"])
        fault("active_pointer")
        marker("restart")
    elif time.monotonic() > state["deadline"]:
        (BRIDGE / "stop").unlink(missing_ok=True)
        manual("old service stop was not confirmed")
    elif stopped.get("ActiveState") not in {"active", "activating", "deactivating"} \
            and not (BRIDGE / "stop").exists():
        marker("stop")


def reconcile(state):
    phase = state["phase"]
    if phase == "manual_recovery":
        manual("manual recovery required")
        return
    if phase in TERMINAL:
        if active_digest() != state["current"]:
            manual("committed object mismatch")
        return
    actual = active_digest()
    if actual not in {state["from"], state["to"]}:
        manual("unexpected active digest")
        return
    if phase == "prepared":
        prepared(state)
    elif phase == "stopping":
        stopping(state)
    elif phase == "rollback_intent":
        rollback(state, state["result"])
    elif phase == "trial":
        if state["boot_id"] != boot_id():
            rollback(state, "interrupted trial")
            return
        current = properties("nlup-lab-agent.service")
        health = heartbeat()
        healthy = actual == state["to"] and current.get("ActiveState") == "active" \
            and current.get("InvocationID") != state["old_invocation"] \
            and str(health.get("pid")) == current.get("MainPID") \
            and health.get("digest") == state["to"] \
            and health.get("sequence") == state["to_sequence"] \
            and 0 <= time.time() - health.get("time", 0) < 2
        if healthy:
            if state["stable_since"] is None or state.get("stable_invocation") != current.get("InvocationID"):
                state["stable_since"] = time.monotonic()
                state["stable_sample"] = health["time"]
                state["stable_invocation"] = current.get("InvocationID")
                save(state)
            elif time.monotonic() - state["stable_since"] > 2 \
                    and health["time"] - state["stable_sample"] > 2:
                state.update(current=state["to"], sequence=state["to_sequence"])
                save(state, "committed", "healthy")
        elif state["stable_since"] is not None:
            state["stable_since"] = None
            save(state)
        if state["phase"] == "trial" and time.monotonic() > state["deadline"]:
            rollback(state, "trial health unconfirmed")


def main():
    if os.geteuid() != pwd.getpwnam("nlup-updater").pw_uid:
        raise RuntimeError("coordinator requires its dedicated unprivileged UID")
    os.umask(0o077)
    is_boot = sys.argv[1:] == ["--boot"]
    with (PRIVATE / "lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | (0 if is_boot else fcntl.LOCK_NB))
        except BlockingIOError:
            return
        if is_boot:
            clear_bridge_requests()
        if (PRIVATE / "manual.json").exists():
            if is_boot:
                raise SystemExit(1)
            return
        try:
            state = decode(read_file(JOURNAL, 16384))
            if not isinstance(state, dict) or type(state.get("schema")) is not int \
                    or state["schema"] != 1 or state.get("phase") not in PHASES:
                raise ValueError("invalid journal schema")
        except (OSError, ValueError, KeyError, TypeError):
            manual("journal invalid")
            if is_boot:
                raise SystemExit(1)
            return
        try:
            reconcile(state)
        except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError):
            manual("recovery state inconsistent")
            if is_boot:
                raise SystemExit(1)
            return
        if (PRIVATE / "manual.json").exists():
            if is_boot:
                raise SystemExit(1)
            return
        try:
            request = INBOX / "request.json"
            if request.exists():
                packet = decode(read_file(request))
                request.unlink()
                if state["phase"] not in TERMINAL:
                    raise ValueError("upgrade already pending")
                attempts = [stamp for stamp in state.get("attempts", []) if time.time() - stamp < 60]
                state["attempts"] = (attempts + [time.time()])[-16:]
                save(state)
                if len(attempts) >= 12:
                    raise ValueError("local attempt rate exceeded")
                prepare(state, packet["job"])
        except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError):
            atomic_json(PRIVATE / "rejection.json", {"result": "input_or_state_rejected"})


if __name__ == "__main__":
    main()
