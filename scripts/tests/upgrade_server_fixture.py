#!/usr/bin/env python3
"""A release fixture with real SQLite migrations and a real loopback readiness endpoint."""

import http.server
import os
from pathlib import Path
import signal
import sqlite3
import sys
import tomllib

VERSION = "@VERSION@"
SCENARIO = "@SCENARIO@"
ROOT = Path("@ROOT@")


def event(value):
    with (ROOT / "events").open("a") as output:
        output.write(value + "\n")


def configuration():
    path = Path(sys.argv[sys.argv.index("--config") + 1])
    return path, tomllib.loads(path.read_text())


def manifest(path, config):
    server = config["server"]
    paths = set()
    for filename in (path, server["node_registry_path"], server["snapshot_path"],
                     config.get("geoip", {}).get("database_path", "data/geoip.mmdb")):
        filename = Path(filename)
        paths.update((filename.absolute(), filename.resolve()))
    for database in (server["history_db_path"], config.get("audit", {}).get("db_path", "data/audit.sqlite3")):
        paths.add(Path(database).absolute())
        paths.update(Path(str(Path(database).resolve()) + suffix)
                     for suffix in ("", "-wal", "-shm", "-journal"))
    print("nodelite-upgrade-manifest-v1")
    print(f"version={VERSION}")
    print(f"ready_url=http://{server['listen']}/readyz")
    for item in sorted(paths):
        print(f"path={item}")


def check_old_database(database):
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    try:
        rows = connection.execute("SELECT value FROM samples").fetchall()
        schema = connection.execute("PRAGMA user_version").fetchone()[0]
        if rows != [("before-upgrade",)] or schema != 1:
            raise ValueError("incompatible database")
    except (sqlite3.Error, ValueError):
        event("OLD_STARTED_WITH_INCOMPATIBLE_DATA")
        raise
    finally:
        connection.close()


def migrate(database):
    connection = sqlite3.connect(database)
    connection.executescript("""
        DROP TABLE samples;
        CREATE TABLE migrated_samples(value TEXT);
        INSERT INTO migrated_samples VALUES ('after-upgrade');
        PRAGMA user_version = 2;
    """)
    connection.close()
    # A failed release can leave sidecars that were absent from the old installation.
    if SCENARIO != "success":
        Path(str(database) + "-journal").write_bytes(b"failed-migration-sidecar")
    event("database-migrated")


def corrupt_backup():
    backup = next((ROOT / "install/.upgrade-backups").glob("upgrade.*"))
    for entry in backup.glob("*.path"):
        if entry.read_text().strip().endswith("history.sqlite3"):
            entry.with_suffix(".tar").write_bytes(b"corrupted-backup")
            return
    raise RuntimeError("history backup missing")


def run_server(config):
    database = Path(config["server"]["history_db_path"])
    if VERSION == "1.0.0":
        check_old_database(database)
        event("old-started-compatible")
    else:
        event("new-started")
        if SCENARIO in {"success", "migrate_fail", "corrupt_backup", "restore_fail", "signal", "symlink"}:
            migrate(database)
            # Production persistence replaces directory entries, including existing symlinks.
            for path, content in (
                (config["server"]["node_registry_path"], b'{"nodes":["changed"]}'),
                (config["server"]["snapshot_path"], b'{"after":true}'),
                (config["geoip"]["database_path"], b"updated-geoip-database"),
            ):
                temporary = Path(str(path) + ".tmp")
                temporary.write_bytes(content)
                temporary.replace(path)
        if SCENARIO == "corrupt_backup":
            corrupt_backup()
        if SCENARIO in {"crash", "migrate_fail", "symlink"}:
            return 17

    host, port = config["server"]["listen"].rsplit(":", 1)
    status = 200
    if VERSION != "1.0.0" and SCENARIO in {"not_ready", "corrupt_backup", "restore_fail", "signal"}:
        status = 503

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            event(f"probe-{VERSION}-{status}")
            self.send_response(status)
            if VERSION != "1.0.0":
                self.send_header("X-NodeLite-Version", "8.8.8" if SCENARIO == "wrong_version" else VERSION)
            self.send_header("Content-Length", "2")
            self.end_headers()
            self.wfile.write(b"{}")

        def log_message(self, *_args):
            pass

    class Server(http.server.HTTPServer):
        allow_reuse_address = True

    signal.signal(signal.SIGTERM, lambda *_args: sys.exit(0))
    signal.signal(signal.SIGINT, lambda *_args: sys.exit(0))
    with Server((host, int(port)), Handler) as server:
        server.serve_forever(poll_interval=0.05)
    return 0


if __name__ == "__main__":
    if "--version" in sys.argv:
        print(f"nodelite-server {VERSION}")
        sys.exit(0)
    config_path, parsed = configuration()
    if "upgrade-manifest" in sys.argv:
        manifest(config_path, parsed)
        sys.exit(0)
    marker = ROOT / "running"
    marker.write_text(str(os.getpid()))
    try:
        sys.exit(run_server(parsed))
    finally:
        marker.unlink(missing_ok=True)
