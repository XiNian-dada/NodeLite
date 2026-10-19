"""Explicit administrator bundle: public trust plus reviewed local programs and fixed units."""

from pathlib import Path

from .io import LIMIT, create, digest, directory, json_bytes, read
from .metadata import now, release_scope
from .repository import current, locked


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
