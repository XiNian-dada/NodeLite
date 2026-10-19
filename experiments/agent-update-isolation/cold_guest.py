#!/usr/bin/env python3
"""Power-loss checkpoints using released Agent and full Server CLI, VM-only."""

import json
from pathlib import Path
import re
import sys
import threading
import time

from cold_support import COLD, CONFIG, NEW, OLD, UNITS, api, online, server_link, setup, stop_all, wait_for
from lab_io import JOURNAL, PRIVATE, ROOT, atomic_bytes, atomic_json, digest
from real_guest import BINARIES, probe, request
from setup_guest import LIB, guard, run
from test_guest import systemctl


def refresh():
    return api("/api/nodes/cold-node/refresh-token", {})


def arm(point):
    original = CONFIG.read_bytes()
    if point in {"before_delivery", "lost_ack"}:
        expected = "refresh_token_response" if point == "before_delivery" else "refresh_token_ack"
        atomic_json(COLD / "gate.json", {"type": expected, "reached": False})

        def trigger():
            try:
                refresh()
            except Exception:
                pass

        threading.Thread(target=trigger, daemon=True).start()
        wait_for(lambda: json.loads((COLD / "gate.json").read_text())["reached"], 15)
        assert (CONFIG.read_bytes() != original) == (point == "lost_ack"), "wrong token write window"
    else:
        assert refresh()["ok"], "real refresh failed"
        assert CONFIG.read_bytes() != original, "rotation was not durably saved"
        if point == "trial_rotated":
            request()
            online(NEW)
            atomic_bytes(COLD / "freeze-health", b"hold\n")
            prior = CONFIG.read_bytes()
            assert refresh()["ok"] and CONFIG.read_bytes() != prior
            assert json.loads(JOURNAL.read_text())["phase"] == "trial"
        else:
            atomic_bytes(Path("/etc/nlup-lab/fault"), point.encode(), 0o644)
            request()
            wait_for(lambda: (PRIVATE / "checkpoint.json").exists(), 15)
            checkpoint = json.loads((PRIVATE / "checkpoint.json").read_text())
            assert checkpoint["point"] == point
    before = {"point": point, "config_sha256": digest(CONFIG.read_bytes()),
              "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
              "phase": json.loads(JOURNAL.read_text())["phase"], "process": probe()}
    atomic_json(COLD / "before.json", before)
    return {"point": point, "phase": before["phase"], "token_changed": CONFIG.read_bytes() != original}


def after():
    before = json.loads((COLD / "before.json").read_text())
    boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    assert boot != before["boot_id"], "guest did not cold boot"
    Path("/etc/nlup-lab/fault").unlink(missing_ok=True)
    point = before["point"]
    expected_phase = "stable" if point in {"before_delivery", "lost_ack"} else "rolled_back"
    wait_for(lambda: json.loads(JOURNAL.read_text())["phase"] == expected_phase)
    online(OLD)
    time.sleep(3)
    online(OLD)
    assert digest(CONFIG.read_bytes()) == before["config_sha256"], "cold recovery reverted credentials"
    # This is the ordinary released read-only preflight, not a harness verifier.
    run("runuser", "-u", "nlup-agent", "--", str(LIB / "credential-check"),
        "--check-credentials", "--config", str(CONFIG))
    assert not (PRIVATE / "manual.json").exists(), "unexpected manual recovery"
    return {"point": point, "passed": True, "new_boot": True, "phase": expected_phase,
            "agent_version": OLD, "config_preserved": True, "credential_check": True}


def restart(upgrade=False):
    # v3.0.3 Server returns before durable ACK: exercise that guarantee only after migration.
    if not upgrade:
        assert refresh()["ok"], "pre-restart rotation failed"
    latest = CONFIG.read_bytes()
    systemctl("stop", "nlup-lab-server.service")
    if upgrade:
        server_link("3.0.4")
    systemctl("start", "nlup-lab-server.service")
    online(OLD)
    assert CONFIG.read_bytes() == latest, "server restart changed saved credentials"
    assert refresh()["ok"] and CONFIG.read_bytes() != latest, "post-restart rotation was not durable"
    return {"passed": True, "scenario": "server_upgrade" if upgrade else "server_restart",
            "credential_reloaded": True, "post_restart_rotation": True}


def inventory():
    sources = [LIB / name for name in ("updater.py", "lab_io.py", "cold_proxy.py", "cold_support.py")]
    sources.extend(Path("/etc/systemd/system") / name for name in UNITS)
    sources.append(Path(__file__))
    binaries = [BINARIES / name for name in ("agent-old", "agent-new", "server-3.0.3", "server-3.0.4")]
    binaries.extend(LIB / name for name in ("server-3.0.3", "server-3.0.4", "credential-check"))
    return {"source_sha256": {str(path): digest(path.read_bytes()) for path in sources},
            "binary_sha256": {str(path): digest(path.read_bytes()) for path in binaries}}


def health_diagnostics():
    result = {"process": probe(), "monotonic": time.monotonic(), "wall": time.time()}
    for name, path in (("manual", PRIVATE / "manual.json"), ("journal", JOURNAL),
                       ("heartbeat", ROOT / "status/heartbeat.json")):
        result[name] = json.loads(path.read_text()) if path.exists() else None
    try:
        status = api("/api/nodes/cold-node")
        result["reported"] = {"online": status["online"], "version": status["identity"]["agent_version"],
                              "sample": (status.get("snapshot") or {}).get("collected_at")}
    except (OSError, RuntimeError) as error:
        result["reported"] = type(error).__name__
    return result


def main():
    guard()
    action = sys.argv[1]
    if action == "setup":
        setup(sys.argv[2] if len(sys.argv) > 2 else "3.0.4")
        result = {"ready": True}
    elif action == "arm":
        result = arm(sys.argv[2])
    elif action == "after":
        result = after()
    elif action == "restart":
        result = restart(len(sys.argv) > 2 and sys.argv[2] == "upgrade")
    elif action == "halt":
        stop_all()
        result = {"stopped": True}
    elif action == "inventory":
        result = inventory()
    elif action == "health-diagnose":
        result = health_diagnostics()
    elif action == "diagnose":
        output = run("journalctl", "-u", "nlup-lab-server.service", "--no-pager", "-n", "12").stdout
        secrets = [json.loads((COLD / "admin.json").read_text())["password"]]
        if CONFIG.exists():
            secrets.extend(re.findall(r'token\s*=\s*"([^"]+)"', CONFIG.read_text()))
        for secret in secrets:
            output = output.replace(secret, "[redacted]")
        result = {"redacted_server_diagnostics": output}
    else:
        raise ValueError("unsupported isolated cold test action")
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
