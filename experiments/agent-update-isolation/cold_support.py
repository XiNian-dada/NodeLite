"""Trusted lab observer and full production CLI server provisioning inside the VM."""

import base64
from datetime import datetime
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import time
import urllib.error
import urllib.request

from lab_io import ROOT, atomic_bytes, atomic_json
from real_guest import BINARIES, halt, own, probe, provision
from setup_guest import LIB, directory, guard, run, unit
from test_guest import systemctl

COLD = ROOT / "cold"
SERVER = ROOT / "server"
UNITS = ("nlup-lab-observer.service", "nlup-lab-proxy.service", "nlup-lab-server.service")
CONFIG = ROOT / "agent/agent.toml"
OLD, NEW = "3.0.4", "lab-next"


class LabApiError(RuntimeError):
    pass


def api(path, body=None):
    auth = json.loads((COLD / "admin.json").read_text())
    basic = base64.b64encode(f"lab:{auth['password']}".encode()).decode()
    headers = {"Authorization": "Basic " + basic, "Content-Type": "application/json"}
    data = None if body is None else json.dumps({**body, "current_password": auth["password"]}).encode()
    request = urllib.request.Request("http://127.0.0.1:18088" + path, data=data, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=25) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        detail = error.read(1024).decode(errors="replace").replace(auth["password"], "[redacted]")
        raise LabApiError(f"lab API {path} rejected {error.code}: {detail}") from None


def wait_for(check, seconds=35):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            result = check()
            if result:
                return result
        except (OSError, ValueError, KeyError, LabApiError):
            pass
        time.sleep(0.1)
    raise RuntimeError("cold-boot lab condition exceeded deadline")


def collection_time(value):
    # Python 3.10 cannot parse chrono's nine-digit fractional seconds directly.
    match = re.fullmatch(r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.(\d{1,9}))?Z", value)
    if not match:
        raise ValueError("invalid lab collection timestamp")
    fraction = (match.group(2) or "0")[:6].ljust(6, "0")
    return datetime.fromisoformat(match.group(1) + "." + fraction + "+00:00").timestamp()


def online(version):
    def check():
        process = probe()
        status = api("/api/nodes/cold-node")
        if not status.get("snapshot"):
            return False
        sample = collection_time(status["snapshot"]["collected_at"])
        health = json.loads((ROOT / "status/heartbeat.json").read_text())
        return process["pid"] > 0 and status["online"] and status["identity"]["agent_version"] == version \
            and 0 <= time.time() - sample < 2 and health["pid"] == process["pid"] \
            and health["digest"] == process["digest"] and 0 <= time.time() - health["time"] < 2
    wait_for(check)


def observe():
    guard()
    invocation, first_seen = None, time.time()
    while True:
        try:
            process = probe()
            if process.get("InvocationID") != invocation:
                invocation, first_seen = process.get("InvocationID"), time.time()
            status = api("/api/nodes/cold-node")
            sample = collection_time(status["snapshot"]["collected_at"])
            identities = json.loads((COLD / "identities.json").read_text())
            sequence = 1 if process.get("digest") == identities["old"] else 2
            version = OLD if sequence == 1 else NEW
            if process.get("digest") in identities.values() and process["pid"] > 0 \
                    and process["ActiveState"] == "active" and status["online"] \
                    and status["identity"]["agent_version"] == version and sample > first_seen \
                    and 0 <= time.time() - sample < 2 and not (COLD / "freeze-health").exists():
                path = ROOT / "status/heartbeat.json"
                # The observer's primary group and mode must be correct before publication.
                atomic_bytes(path, json.dumps({"pid": process["pid"], "digest": process["digest"],
                             "sequence": sequence, "time": sample}).encode(), 0o640)
        except (OSError, ValueError, KeyError, TypeError, LabApiError):
            pass
        time.sleep(0.15)


def stop_all():
    halt()
    systemctl("stop", *UNITS, check=False)
    systemctl("disable", *UNITS, check=False)


def server_link(version):
    link = LIB / "server-active"
    temporary = LIB / ".server-next"
    temporary.unlink(missing_ok=True)
    temporary.symlink_to("server-" + version)
    os.replace(temporary, link)


def install_units():
    for name, command in (("server", f"{LIB}/server-active --config {SERVER}/server.toml"),
                          ("proxy", f"/usr/bin/python3 {LIB}/cold_proxy.py"),
                          ("observer", f"/usr/bin/python3 {LIB}/cold_support.py")):
        user = "nlup-server" if name == "server" else "root"
        group = "nlup-read" if name == "observer" else user
        capabilities = "CAP_DAC_OVERRIDE CAP_SYS_PTRACE" if name == "observer" else ""
        clear = {"proxy": f"ExecStartPre=/usr/bin/rm -f {COLD}/gate.json\n",
                 "observer": f"ExecStartPre=/usr/bin/rm -f {COLD}/freeze-health\n"}.get(name, "")
        unit(f"{name}.service", f"""[Unit]
Description=Disposable full-CLI test {name}
After=network.target
[Service]
User={user}
Group={group}
WorkingDirectory={SERVER}
Environment=RUST_LOG=off PYTHONDONTWRITEBYTECODE=1 PYTHONPATH={LIB}/python-libs
{clear}ExecStart={command}
Restart=always
RestartSec=300ms
UMask=0077
NoNewPrivileges=yes
CapabilityBoundingSet={capabilities}
AmbientCapabilities=
[Install]
WantedBy=multi-user.target
""")
    systemctl("daemon-reload")
    run("systemd-analyze", "verify", *(f"/etc/systemd/system/{unit}" for unit in UNITS))


def setup(version="3.0.4"):
    guard()
    stop_all()
    if not run("getent", "passwd", "nlup-agent").stdout:
        raise RuntimeError("base experiment must be provisioned first")
    import subprocess
    if subprocess.run(["id", "nlup-server"], capture_output=True).returncode:
        run("useradd", "--system", "--no-create-home", "--shell", "/usr/sbin/nologin", "nlup-server")
    directory(COLD, "root", "root", 0o700)
    directory(SERVER, "nlup-server", "nlup-server", 0o700)
    for path in (COLD / "gate.json", COLD / "freeze-health"):
        path.unlink(missing_ok=True)
    for name in ("cold_proxy.py", "cold_support.py", "real_guest.py", "test_guest.py", "setup_guest.py"):
        shutil.copyfile(Path(__file__).parent / name, LIB / name)
        (LIB / name).chmod(0o555)
    for release in ("3.0.3", "3.0.4"):
        shutil.copyfile(BINARIES / f"server-{release}", LIB / f"server-{release}")
        (LIB / f"server-{release}").chmod(0o555)
    shutil.copyfile(BINARIES / "agent-old", LIB / "credential-check")
    (LIB / "credential-check").chmod(0o555)
    server_link(version)
    password = "Aa!9" + secrets.token_urlsafe(32)
    atomic_json(COLD / "admin.json", {"password": password})
    # No external GeoIP requests or production URLs are permitted from this guest.
    config = f'''[server]
listen = "127.0.0.1:18088"
public_base_url = "http://127.0.0.1:18089"
insecure_allow_http = true
node_registry_path = "{SERVER}/registry.json"
history_db_path = "{SERVER}/history.sqlite3"
snapshot_path = "{SERVER}/snapshot.json"
ping_interval_secs = 1
[auth]
username = "lab"
password = "{password}"
[audit]
db_path = "{SERVER}/audit.sqlite3"
[geoip]
enabled = false
'''
    atomic_bytes(SERVER / "server.toml", config.encode())
    own(SERVER / "server.toml", "nlup-server", group="nlup-server")
    result = run("runuser", "-u", "nlup-server", "--", str(LIB / "server-active"),
                 "--config", str(SERVER / "server.toml"), "issue-node", "--node-id", "cold-node")
    agent = result.stdout.split("# agent.toml\n", 1)[1].split("# install command", 1)[0]
    agent = agent.replace("report_interval_secs = 5", "report_interval_secs = 1")
    install_units()
    systemctl("reset-failed", *UNITS, check=False)
    systemctl("enable", "--now", *UNITS)
    wait_for(lambda: api("/api/overview"))
    identities = provision(agent)
    atomic_json(COLD / "identities.json", identities)
    online(OLD)
    return identities


if __name__ == "__main__":
    observe()
