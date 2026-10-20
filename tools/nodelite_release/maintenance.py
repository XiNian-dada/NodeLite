"""Pinned readiness checks and recoverable retirement of old public generations."""

from contextlib import contextmanager
import os
from pathlib import Path
import re
import secrets
import stat
import time

from .io import LIMIT, digest, owned_directory, read
from .metadata import now
from .repository import META_NAME, TARGET_NAME, current, locked

WINDOWS = {"root": 30 * 86400, "targets": 14 * 86400, "snapshot": 2 * 86400, "timestamp": 6 * 3600}


@contextmanager
def directory_fd(path, mode, parent=None):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
    try:
        info = os.fstat(fd)
        if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != mode:
            raise ValueError("archive directory ownership or mode changed")
        yield fd
    finally:
        os.close(fd)


def retire(name, target, source_fd, archive_fd):
    with directory_fd(name, 0o555, source_fd) as generation_fd:
        # Pin the inode: an ancestor or archive-entry substitution must never redirect chmod.
        os.fchmod(generation_fd, 0o755)
        try:
            os.rename(name, target, src_dir_fd=source_fd, dst_dir_fd=archive_fd)
        finally:
            os.fchmod(generation_fd, 0o555)
            os.fsync(generation_fd)
        moved = os.stat(target, dir_fd=archive_fd, follow_symlinks=False)
        held = os.fstat(generation_fd)
        if (moved.st_dev, moved.st_ino) != (held.st_dev, held.st_ino):
            raise ValueError("archive entry changed; manual inspection required")
        os.fsync(archive_fd)
        os.fsync(source_fd)


def inventory(generation):
    owned_directory(generation)
    if stat.S_IMODE(generation.lstat().st_mode) != 0o555 \
            or {item.name for item in generation.iterdir()} != {"metadata", "targets"}:
        raise ValueError("unexpected or incomplete public generation")
    total = 0
    for name, pattern, limit in (("metadata", META_NAME, 2 * 1024 * 1024), ("targets", TARGET_NAME, LIMIT)):
        folder = generation / name
        owned_directory(folder)
        if stat.S_IMODE(folder.lstat().st_mode) != 0o555:
            raise ValueError("public generation is not immutable")
        for count, item in enumerate(folder.iterdir(), 1):
            if count > 8192 or not pattern.fullmatch(item.name):
                raise ValueError("public inventory exceeds supported scope")
            info = item.lstat()
            if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o444:
                raise ValueError("public inventory ownership/mode rejected")
            data = read(item, limit)
            if name == "targets" and digest(data) != item.name[:64]:
                raise ValueError("historical target digest mismatch")
            total += len(data)
    return total


def check(destination, root_sha256):
    if not re.fullmatch(r"[0-9a-f]{64}", root_sha256):
        raise ValueError("independent root fingerprint required")
    with locked(destination, shared=True):
        generation, chain, documents, custom, info = current(destination)
        fingerprints = [digest(read(generation / "metadata" / f"{root.signed.version}.root.json")) for root in chain]
        if root_sha256 not in fingerprints:
            raise ValueError("repository does not contain the independently trusted root")
        sizes = inventory(generation)
        deadlines, warnings = {}, []
        for document in [chain[-1], *documents]:
            role = document.signed.type
            remaining = int((document.signed.expires - now()).total_seconds())
            deadlines[role] = {"version": document.signed.version, "remaining_seconds": remaining}
            if remaining <= WINDOWS[role]:
                warnings.append(role + (":expired" if remaining <= 0 else ":renew_soon"))
        count = len(list((Path(destination) / "generations").iterdir()))
        if count >= 48:
            warnings.append("repository:archive_old_generations")
        return {"ready": not warnings, "warnings": warnings, "expiry": deadlines, "generations": count,
                "current": generation.name, "current_root_sha256": fingerprints[-1], "public_bytes": sizes,
                "channel": custom["channel"], "target": custom["target"], "sequence": custom["release_sequence"],
                "agent_sha256": info.hashes["sha256"]}


def collect(destination, archive, keep=3, minimum_age=3600, apply=False):
    if type(keep) is not int or not 2 <= keep <= 63 or type(minimum_age) is not int or minimum_age < 3600:
        raise ValueError("retain 2..63 generations and at least one hour of reader grace")
    destination, archive = Path(destination), Path(archive)
    owned_directory(archive, private=True)
    if archive.resolve().is_relative_to(destination.resolve()) or destination.resolve().is_relative_to(archive.resolve()):
        raise ValueError("archive must be outside and independent of the public repository")
    folder = destination / "generations"
    with locked(destination), directory_fd(archive, 0o700) as archive_fd, \
            directory_fd(folder, 0o755) as source_fd:
        live, _, _, _, _ = current(destination)
        inventory(live)
        if os.fstat(source_fd).st_dev != os.fstat(archive_fd).st_dev:
            raise ValueError("archive must share the filesystem for atomic, recoverable renames")
        candidates = []
        for entry in folder.iterdir():
            if not re.fullmatch(r"[0-9a-f]{32}", entry.name):
                raise ValueError("unknown generation: manual inspection required")
            inventory(entry)
            if entry != live:
                candidates.append(entry)
        candidates.sort(key=lambda item: (item.stat().st_mtime_ns, item.name), reverse=True)
        eligible = [item for item in candidates[keep - 1:] if time.time() - item.stat().st_mtime >= minimum_age]
        moved = []
        for item in eligible:
            target = archive / (item.name + "-" + secrets.token_hex(8))
            if apply:
                retire(item.name, target.name, source_fd, archive_fd)
            moved.append({"generation": item.name, "archive": target.name if apply else None})
        return {"applied": apply, "current": live.name, "retired": moved, "deleted": False,
                "trust_history_retained_in_current": True}
