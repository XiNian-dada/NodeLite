"""Explicit administrator bundle: public trust plus reviewed local programs and fixed units."""

import io
from pathlib import Path
import tarfile
import tempfile

from .io import LIMIT, create, digest, directory, json_bytes, read
from .metadata import now, release_scope
from .repository import current, locked

# Only the stdlib migration/recovery runtime is shipped to nodes, never signing tools.
SOURCES = ("tools/__init__.py", "tools/nodelite_release/__init__.py", "tools/nodelite_release/io.py") + tuple(
    "tools/nodelite_migrate/" + name for name in
    ("__init__.py", "__main__.py", "common.py", "bundle.py", "transaction.py", "audit.py", "installer.py"))


def pack(repository, updater, units, output, bootstrap_version=None):
    with locked(repository, shared=True):
        return pack_locked(repository, updater, units, output, bootstrap_version)


def pack_locked(repository, updater, units, output, bootstrap_version):
    generation, chain, _, custom, info = current(repository, fresh=True)
    if chain[-1].signed.expires <= now():
        raise ValueError("root expired")
    bootstrap_version = bootstrap_version if bootstrap_version is not None else len(chain)
    if type(bootstrap_version) is not int or not 1 <= bootstrap_version <= len(chain):
        raise ValueError("bootstrap root must be part of the verified history")
    output = directory(output)
    files = {"root.json": read(generation / "metadata" / f"{bootstrap_version}.root.json"),
             "nodelite-updater": read(updater, LIMIT, allow_aliased=True),
             "nodelite-agent": read(generation / "targets" / (info.hashes["sha256"] + "." + release_scope(custom)), LIMIT)}
    if files["nodelite-updater"][:4] != b"\x7fELF" or b"/etc/nlup-lab-guest" in files["nodelite-updater"]:
        raise ValueError("updater must be a normal ELF without lab fault hooks")
    info.verify_length_and_hashes(files["nodelite-agent"])
    expected = {"nodelite-agent.service", "nodelite-agent-preflight.service"}
    expected.update("nodelite-updater-" + name for name in ("apply.service", "boot.service", "reconcile.service", "reconcile.timer",
        "preflight.service", "preflight.path", "stop.service", "stop.path", "restart.service", "restart.path"))
    if {path.name for path in Path(units).iterdir()} != expected:
        raise ValueError("fixed systemd unit inventory mismatch")
    directory(output / "systemd")
    for name in sorted(expected):
        files["systemd/" + name] = read(Path(units) / name, 16384)
    for name, content in files.items():
        create(output / name, content, 0o555 if name in {"nodelite-agent", "nodelite-updater"} else 0o644)
    manifest = json_bytes({"schema": 1, "channel": custom["channel"], "target": custom["target"],
        "baseline_sequence": custom["release_sequence"], "minimum_sequence": custom["min_rollback_sequence"],
        "files": {name: digest(content) for name, content in files.items()}})
    create(output / "manifest.json", manifest)
    return {"bundle_sha256": digest(manifest), "root_sha256": digest(files["root.json"]),
            "baseline_sha256": info.hashes["sha256"], "private_keys_included": False}


def pack_installer(repository, updater, units, output, metadata_url, targets_url, bootstrap_version=None):
    from tools.nodelite_migrate import bundle as migration

    with tempfile.TemporaryDirectory() as temporary:
        staged = Path(temporary) / "bundle"
        pins = pack(repository, updater, units, staged, bootstrap_version)
        manifest, files = migration.inspect(staged, pins["bundle_sha256"], pins["root_sha256"])
        migration.policy(manifest, files, metadata_url, targets_url)
        files["manifest.json"] = read(staged / "manifest.json")
        files["enrollment.json"] = json_bytes({"schema": 1, "bundle_sha256": pins["bundle_sha256"],
            "root_sha256": pins["root_sha256"], "metadata_url": metadata_url, "targets_url": targets_url})
        source = Path(__file__).resolve().parents[2]
        files.update({name: read(source / name) for name in SOURCES})
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
            for name, data in sorted(files.items()):
                entry = tarfile.TarInfo(name)
                entry.size, entry.mode = len(data), 0o600
                archive.addfile(entry, io.BytesIO(data))
        payload = buffer.getvalue()
        if len(payload) > 2 * LIMIT + 1024 * 1024:
            raise ValueError("enrollment archive too large")
        create(output, payload)
        checksum = digest(payload)
        create(str(output) + ".sha256", (checksum + "  " + Path(output).name + "\n").encode())
        return {**pins, "archive_sha256": checksum, "target": manifest["target"]}
