#!/usr/bin/env python3
"""Exercise the entire installer against disposable releases, real HTTP and SQLite.

Use --systemd as root on Linux to replace the process controller with actual units.
No production service, executable, configuration or database path is used.
"""

import argparse
import fcntl
import hashlib
import os
from pathlib import Path
import secrets
import shutil
import signal
import socket
import sqlite3
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request

SCRIPTS = Path(__file__).resolve().parent
SCENARIOS = ["crash", "not_ready", "wrong_version", "success", "migrate_fail", "corrupt_backup",
             "restore_fail", "backup_fail", "signal", "bad_binary_digest", "bad_helper_digest",
             "candidate_version", "inactive", "symlink", "locked"]


def wait_until(predicate, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise AssertionError("fixture did not reach the expected state")


def ready(port):
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/readyz", timeout=0.2) as response:
            return response.status == 200
    except (OSError, urllib.error.URLError):
        return False


class Installation:
    def __init__(self, scenario, systemd):
        self.scenario = scenario
        self.systemd = systemd
        self.directory = tempfile.TemporaryDirectory(prefix="nodelite-upgrade-test-", dir="/run" if systemd else None)
        self.root = Path(self.directory.name).resolve()
        self.install = self.root / "install"
        self.binary = self.root / "bin/nodelite-server"
        self.name = self.root.name
        self.unit = Path("/etc/systemd/system") / f"{self.name}.service" if systemd else self.root / "server.service"
        self.config = self.install / "config/server.toml"
        self.data = self.install / "data"
        self.release = self.root / "release"
        self.wrappers = self.root / "commands"
        self.environment = os.environ.copy()
        self.process = None
        self.lock_file = None
        self.old_binary = b""
        self.old_config = b""
        self.original_files = {}

    def command(self, *args, check=True):
        return subprocess.run(["systemctl", *args], env=self.environment, check=check,
                              capture_output=True, text=True, timeout=15)

    def events(self):
        path = self.root / "events"
        return path.read_text() if path.exists() else ""

    def populate_release(self):
        source = (SCRIPTS / "tests/upgrade_server_fixture.py").read_text()

        def binary(version):
            return source.replace('"@VERSION@"', repr(version)).replace('"@SCENARIO@"', repr(self.scenario)) \
                .replace('"@ROOT@"', repr(str(self.root))).encode()

        self.old_binary = binary("1.0.0")
        self.binary.write_bytes(self.old_binary)
        self.binary.chmod(0o755)
        version = "9.9.8" if self.scenario == "candidate_version" else "9.9.9"
        for target in ("aarch64", "x86_64"):
            (self.release / f"nodelite-server-{target}-unknown-linux-musl").write_bytes(binary(version))
        for helper in ("install-server-config.sh", "install-server-upgrade.sh"):
            shutil.copyfile(SCRIPTS / helper, self.release / helper)
        checksum_lines = []
        for path in sorted(self.release.iterdir()):
            checksum_lines.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n")
        (self.release / "SHA256SUMS.txt").write_text("".join(checksum_lines))
        if self.scenario == "bad_binary_digest":
            for path in self.release.glob("nodelite-server-*"):
                path.write_bytes(b"wrong-checksum")
        if self.scenario == "bad_helper_digest":
            (self.release / "install-server-upgrade.sh").write_text("exit 0\n")

    def populate_commands(self):
        self.environment.update({
            "NODELITE_INSTALL_TEST_ROOT": str(self.root),
            "NODELITE_INSTALL_TEST_UNIT": str(self.unit),
            "NODELITE_INSTALL_TEST_SCENARIO": self.scenario,
            "NODELITE_INSTALL_REAL_CURL": shutil.which("curl"),
            "NODELITE_INSTALL_REAL_TAR": shutil.which("tar"),
            "NODELITE_SERVER_MODE": "upgrade",
            "NODELITE_SERVER_VERSION": "v9.9.9",
            "NODELITE_SERVER_BASE_URL": "https://example.invalid/releases/v9.9.9",
            "NODELITE_SERVER_READY_TIMEOUT_SECS": "4",
            "TMPDIR": str(self.root),
            "PATH": f"{self.wrappers}{os.pathsep}{os.environ['PATH']}",
        })
        commands = ["curl", "tar"]
        if not self.systemd:
            commands.extend(["id", "chown", "install", "systemctl"])
        for utility in ("timeout", "flock"):
            if not shutil.which(utility):
                commands.append(utility)
        source = (SCRIPTS / "tests/upgrade_command_fixture.py").read_bytes()
        for command in commands:
            path = self.wrappers / command
            path.write_bytes(source)
            path.chmod(0o755)

    def seed_database(self):
        database = self.data / "history.sqlite3"
        # Exiting without close leaves committed data in a real WAL for backup/rollback.
        subprocess.run([sys.executable, "-c", """
import os, sqlite3, sys
connection = sqlite3.connect(sys.argv[1])
connection.execute('PRAGMA journal_mode=WAL')
connection.execute('PRAGMA wal_autocheckpoint=0')
connection.execute('CREATE TABLE samples(value TEXT)')
connection.execute("INSERT INTO samples VALUES ('before-upgrade')")
connection.execute('PRAGMA user_version=1')
connection.commit()
os._exit(0)
""", str(database)], check=True)
        assert Path(str(database) + "-wal").stat().st_size > 0

    def setup(self):
        for directory in (self.binary.parent, self.config.parent, self.data, self.release, self.wrappers):
            directory.mkdir(parents=True, mode=0o700)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            self.port = sock.getsockname()[1]
        self.populate_release()
        self.populate_commands()
        self.password = secrets.token_urlsafe(24)
        self.config.write_text(f"""[server]
listen = "127.0.0.1:{self.port}"
public_base_url = "https://monitor.example.invalid"
node_registry_path = "{self.config.parent}/server.json"
history_db_path = "{self.data}/history.sqlite3"
snapshot_path = "{self.data}/snapshot.json"

[auth]
username = "viewer"
password = "{self.password}"

[geoip]
enabled = false
database_path = "{self.data}/geoip.mmdb"
""")
        self.config.chmod(0o600)
        self.old_config = self.config.read_bytes()
        for path, content in ((self.config.parent / "server.json", b'{"nodes":[]}'),
                              (self.data / "snapshot.json", b'{"before":true}'),
                              (self.data / "geoip.mmdb", b"original-database")):
            path.write_bytes(content)
            path.chmod(0o600)
        self.seed_database()
        self.unit.write_text(f"""[Unit]
Description=Isolated NodeLite upgrade regression
[Service]
Type=simple
ExecStart={self.binary} --config {self.config}
WorkingDirectory={self.install}
Restart=always
RestartSec=1
TimeoutStopSec=5
[Install]
WantedBy=multi-user.target
""")
        self.original_unit = self.unit.read_bytes()
        if self.scenario == "symlink":
            assets = self.root / "old-assets"
            assets.mkdir(mode=0o700)
            for path in (self.binary, self.unit):
                destination = assets / path.name
                path.rename(destination)
                path.symlink_to(destination)
        source = (SCRIPTS / "install-server.sh").read_text()
        replacements = {
            'SERVICE_NAME="nodelite-server"': f'SERVICE_NAME="{self.name}"',
            'BIN_PATH="/usr/local/bin/nodelite-server"': f'BIN_PATH="{self.binary}"',
            'UNIT_PATH="/etc/systemd/system/${SERVICE_NAME}.service"': f'UNIT_PATH="{self.unit}"',
        }
        for before, after in replacements.items():
            assert source.count(before) == 1
            source = source.replace(before, after)
        self.installer = self.root / "installer.sh"
        self.installer.write_text(source)
        self.command("daemon-reload")
        if self.scenario != "inactive":
            self.command("start", f"{self.name}.service")
            wait_until(lambda: ready(self.port))
        self.before_events = self.events()
        self.original_files = {path: path.read_bytes() for path in self.install.rglob("*") if path.is_file()}

    def run(self):
        output_path = self.root / "installer.log"
        if self.scenario == "locked":
            self.lock_file = Path(str(self.binary) + ".install.lock").open("a")
            fcntl.flock(self.lock_file, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with output_path.open("w") as output:
            self.process = subprocess.Popen(["sh", str(self.installer)], env=self.environment,
                                            stdin=subprocess.DEVNULL, stdout=output, stderr=output)
            if self.scenario == "signal":
                wait_until(lambda: "probe-9.9.9-503" in self.events())
                self.process.send_signal(signal.SIGTERM)
            code = self.process.wait(timeout=45)
        output = output_path.read_text()
        assert self.password not in output, "installer leaked a password"
        events = self.events()[len(self.before_events):]
        assert "OLD_STARTED_WITH_INCOMPATIBLE_DATA" not in events, events
        success = self.scenario in {"success", "inactive"}
        assert (code == 0) == success, output
        assert ("NodeLite server upgraded and restarted." in output) == success, output
        if self.scenario in {"bad_binary_digest", "bad_helper_digest", "candidate_version", "locked"}:
            assert "systemctl-stop" not in events, events
            assert self.config.read_bytes() == self.old_config
            assert self.binary.read_bytes() == self.old_binary
            assert not (self.install / ".upgrade-backups").exists()
            return
        backups = list((self.install / ".upgrade-backups").glob("upgrade.*"))
        assert len(backups) == 1, output
        backup = backups[0]
        assert stat.S_IMODE(backup.stat().st_mode) == 0o700
        assert all(stat.S_IMODE(path.stat().st_mode) == 0o600 for path in backup.iterdir())
        if self.scenario in {"corrupt_backup", "restore_fail"}:
            assert "automatic recovery incomplete" in output, output
            assert self.command("is-active", "--quiet", f"{self.name}.service", check=False).returncode != 0
            assert "old-started-compatible" not in events, events
            assert self.binary.read_bytes() != self.old_binary
            return
        assert self.command("is-active", "--quiet", f"{self.name}.service", check=False).returncode == 0, output
        if success:
            assert self.binary.read_bytes() != self.old_binary
            assert b"history_writer_flush_interval_ms" in self.config.read_bytes()
            assert "Committed: version 9.9.9 is ready." in (backup / "status").read_text()
            self.verify_backup(backup)
        else:
            assert self.config.read_bytes() == self.old_config, output
            assert self.binary.read_bytes() == self.old_binary, output
            assert self.unit.read_bytes() == self.original_unit, output
            if self.scenario == "symlink":
                assert self.binary.is_symlink() and self.unit.is_symlink()
            assert "old-started-compatible" in events, events
            assert not Path(str(self.data / "history.sqlite3") + "-journal").exists()
            for path in (self.config.parent / "server.json", self.data / "snapshot.json", self.data / "geoip.mmdb"):
                assert path.read_bytes() == self.original_files[path]
            connection = sqlite3.connect(f"file:{self.data / 'history.sqlite3'}?mode=ro", uri=True)
            assert connection.execute("SELECT value FROM samples").fetchall() == [("before-upgrade",)]
            assert connection.execute("PRAGMA user_version").fetchone()[0] == 1
            connection.close()

    def verify_backup(self, backup):
        archived = {}
        for index in backup.glob("*.path"):
            original = Path(index.read_text().strip())
            archive = index.with_suffix(".tar")
            if archive.exists():
                with tarfile.open(archive) as content:
                    archived[original] = content.extractfile(str(original)).read()
        assert archived[self.binary] == self.old_binary
        assert archived[self.config] == self.old_config
        for suffix in ("", "-wal"):
            path = Path(str(self.data / "history.sqlite3") + suffix)
            assert archived[path] == self.original_files[path], f"incompatible SQLite backup: {path}"

    def cleanup(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            self.process.wait(timeout=45)
        try:
            self.command("stop", f"{self.name}.service", check=False)
            if self.systemd:
                self.command("disable", f"{self.name}.service", check=False)
                self.unit.unlink(missing_ok=True)
                self.command("daemon-reload")
        finally:
            if self.lock_file:
                self.lock_file.close()
            self.directory.cleanup()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--systemd", action="store_true")
    parser.add_argument("scenarios", nargs="*")
    args = parser.parse_args()
    if args.systemd and (sys.platform != "linux" or os.geteuid() != 0):
        parser.error("--systemd requires a root Linux runner with systemd")
    scenarios = args.scenarios or SCENARIOS
    if any(scenario not in SCENARIOS for scenario in scenarios):
        parser.error(f"scenarios must be selected from: {', '.join(SCENARIOS)}")
    for scenario in scenarios:
        fixture = Installation(scenario, args.systemd)
        try:
            fixture.setup()
            fixture.run()
            print(f"PASS server upgrade: {scenario} ({'systemd' if args.systemd else 'process fixture'})", flush=True)
        except Exception:
            for log in ("installer.log", "server.log", "events"):
                path = fixture.root / log
                if path.exists():
                    print(f"{scenario}: {log}\n{path.read_text()}", file=sys.stderr)
            raise
        finally:
            fixture.cleanup()
    print(f"Server upgrade acceptance: {len(scenarios)} cases passed, zero skipped")


if __name__ == "__main__":
    main()
