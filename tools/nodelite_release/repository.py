"""Verify every role before an immutable static generation becomes the current mirror."""

import fcntl
import os
from pathlib import Path
import re
import secrets

from .io import LIMIT, create, digest, directory, owned_directory, read, sync
from .metadata import load, now, release_scope, root_safe

META_NAME = re.compile(r"(?:[1-9][0-9]*\.(?:root|targets|snapshot)\.json|timestamp\.json)")
TARGET_NAME = re.compile(r"[0-9a-f]{64}\.nodelite-agent-(?:stable|testing)-(?:x86_64|aarch64)-unknown-linux-musl")


def roots(path):
    entries = {}
    for item in Path(path).iterdir():
        match = re.fullmatch(r"([1-9][0-9]*)\.root\.json", item.name)
        if match:
            entries[int(match[1])] = item
    if not 1 <= len(entries) <= 128 or set(entries) != set(range(1, len(entries) + 1)):
        raise ValueError("root history must start at 1 and have no gaps")
    chain = []
    for version in sorted(entries):
        document = load(entries[version])
        root_safe(document.signed)
        if document.signed.version != version:
            raise ValueError("root filename/version mismatch")
        document.verify_delegate("root", document)
        if chain:
            chain[-1].verify_delegate("root", document)
        chain.append(document)
    return chain


def release(root, targets_path, snapshot_path, timestamp_path, artifact=None, fresh=True):
    documents = [load(path) for path in (targets_path, snapshot_path, timestamp_path)]
    for document, role in zip(documents, ("targets", "snapshot", "timestamp")):
        if document.signed.type != role:
            raise ValueError("wrong metadata role")
        root.verify_delegate(role, document)
        if fresh and document.signed.expires <= now():
            raise ValueError("published metadata expired")
    targets, snapshot, timestamp = documents
    versions = {item.signed.version for item in documents}
    if len(versions) != 1 or len(targets.signed.targets) != 1 \
            or set(snapshot.signed.meta) != {"targets.json"}:
        raise ValueError("single-scope repository version contract rejected")
    snapshot.signed.meta["targets.json"].verify_length_and_hashes(read(targets_path))
    timestamp.signed.snapshot_meta.verify_length_and_hashes(read(snapshot_path))
    if snapshot.signed.meta["targets.json"].version != targets.signed.version \
            or timestamp.signed.snapshot_meta.version != snapshot.signed.version:
        raise ValueError("metadata child version mismatch")
    name, info = next(iter(targets.signed.targets.items()))
    custom = info.unrecognized_fields.get("custom", {})
    if release_scope(custom) != name or info.length <= 0 or info.length > LIMIT \
            or set(info.hashes) != {"sha256"}:
        raise ValueError("target name, size or hash contract rejected")
    if artifact is not None:
        info.verify_length_and_hashes(read(artifact, LIMIT))
    return documents, custom, info


def current(destination, fresh=False):
    destination = Path(destination)
    pointer = destination / "current"
    if not pointer.is_symlink() or not re.fullmatch(r"generations/[0-9a-f]{32}", os.readlink(pointer)):
        raise ValueError("current generation missing or unsafe; never reset a repository")
    generation = destination / os.readlink(pointer)
    owned_directory(generation)
    chain = roots(generation / "metadata")
    timestamp = load(generation / "metadata/timestamp.json")
    version = timestamp.signed.version
    paths = [generation / "metadata" / f"{version}.{role}.json" for role in ("targets", "snapshot")]
    documents, custom, info = release(chain[-1], *paths, generation / "metadata/timestamp.json", fresh=fresh)
    artifact = generation / "targets" / (info.hashes["sha256"] + "." + release_scope(custom))
    info.verify_length_and_hashes(read(artifact, LIMIT))
    return generation, chain, documents, custom, info


def publish(roots_path, targets, snapshot, timestamp, artifact, destination):
    chain = roots(roots_path)
    if chain[-1].signed.expires <= now():
        raise ValueError("current root expired")
    documents, custom, info = release(chain[-1], targets, snapshot, timestamp, artifact)
    payload = read(artifact, LIMIT)
    info.verify_length_and_hashes(payload)
    destination = Path(destination)
    if not destination.exists():
        directory(destination, 0o755)
        directory(destination / "generations", 0o755)
    owned_directory(destination)
    owned_directory(destination / "generations")
    lock = os.open(destination / ".lock", os.O_CREAT | os.O_WRONLY | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return publish_locked(destination, chain, documents, custom, info, payload, roots_path,
                              (targets, snapshot, timestamp))
    finally:
        os.close(lock)


def publish_locked(destination, chain, documents, custom, info, payload, roots_path, metadata_paths):
    previous = None
    if (destination / "current").is_symlink() or (destination / "current").exists():
        previous, old_chain, old_documents, old_custom, old_info = current(destination)
        if len(chain) < len(old_chain) or any(new.to_bytes() != old.to_bytes() for new, old in zip(chain, old_chain)):
            raise ValueError("root history downgrade or equivocation")
        if (custom["channel"], custom["target"]) != (old_custom["channel"], old_custom["target"]) \
                or documents[0].signed.version <= old_documents[0].signed.version \
                or custom["release_sequence"] < old_custom["release_sequence"] \
                or custom["min_rollback_sequence"] < old_custom["min_rollback_sequence"]:
            raise ValueError("publication scope, version or floor rollback rejected")
        if custom["release_sequence"] == old_custom["release_sequence"] \
                and (custom != old_custom or info.hashes != old_info.hashes):
            raise ValueError("same-release equivocation rejected")
    elif any((destination / "generations").iterdir()):
        raise ValueError("lost current pointer requires operator recovery")
    if len(list((destination / "generations").iterdir())) >= 64:
        raise ValueError("publication retention quota requires explicit maintenance")
    identity = secrets.token_hex(16)
    generation = directory(destination / "generations" / identity, 0o755)
    for name, pattern, limit in (("metadata", META_NAME, 2 * 1024 * 1024), ("targets", TARGET_NAME, LIMIT)):
        output = directory(generation / name, 0o755)
        if previous:
            for item in (previous / name).iterdir():
                if not pattern.fullmatch(item.name):
                    raise ValueError("unexpected public repository file")
                create(output / item.name, read(item, limit), 0o444)
    for version in range(1, len(chain) + 1):
        put_immutable(generation / "metadata" / f"{version}.root.json", read(Path(roots_path) / f"{version}.root.json"))
    for document, path in zip(documents, metadata_paths):
        name = "timestamp.json" if document.signed.type == "timestamp" else f"{document.signed.version}.{document.signed.type}.json"
        target = generation / "metadata" / name
        if name == "timestamp.json" and target.exists():
            target.unlink()
        put_immutable(target, read(path))
    put_immutable(generation / "targets" / (info.hashes["sha256"] + "." + release_scope(custom)), payload)
    for path in (generation / "metadata", generation / "targets", generation):
        path.chmod(0o555)
        sync(path)
    sync(generation.parent)
    pointer = destination / (".current-" + identity)
    pointer.symlink_to("generations/" + identity)
    os.replace(pointer, destination / "current")
    sync(destination)
    return {"generation": identity, "sequence": custom["release_sequence"], "sha256": info.hashes["sha256"],
            "root_version": len(chain), "bootstrap_sha256": digest(read(Path(roots_path) / "1.root.json"))}


def put_immutable(path, data):
    if path.exists():
        if read(path, LIMIT) != data:
            raise ValueError("immutable publication collision")
    else:
        create(path, data, 0o444)
