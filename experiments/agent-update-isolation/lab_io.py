"""Private inode snapshots and durable state for the disposable updater."""

import hashlib
import json
import os
from pathlib import Path
import secrets
import stat

ROOT = Path("/var/lib/nlup-lab")
CODE = ROOT / "code"
PRIVATE = ROOT / "private"
INBOX = ROOT / "inbox"
BRIDGE = ROOT / "bridge"
JOURNAL = PRIVATE / "journal.json"
LIMIT = 2 * 1024 * 1024


def digest(data):
    return hashlib.sha256(data).hexdigest()


def sync_dir(directory):
    fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic_bytes(path, data, mode=0o600):
    temporary = path.with_name(f".{path.name}.{secrets.token_hex(8)}")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    try:
        os.fchmod(fd, mode)
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        sync_dir(path.parent)
    finally:
        temporary.unlink(missing_ok=True)


def atomic_json(path, value):
    atomic_bytes(path, json.dumps(value, sort_keys=True).encode() + b"\n")


def no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON field")
        result[key] = value
    return result


def decode(data):
    return json.loads(data, object_pairs_hook=no_duplicates)


def bounded_at(directory_fd, name, maximum):
    fd = os.open(name, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW, dir_fd=directory_fd)
    try:
        metadata = os.fstat(fd)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1 or metadata.st_size > maximum:
            raise ValueError("unsafe input file")
        with os.fdopen(fd, "rb", closefd=False) as stream:
            data = stream.read(maximum + 1)
        if len(data) > maximum:
            raise ValueError("oversized input")
        return data
    finally:
        os.close(fd)


def read_file(path, maximum=4096):
    fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        return bounded_at(fd, path.name, maximum)
    finally:
        os.close(fd)


def object_path(identity):
    if not isinstance(identity, str) or len(identity) != 64 or any(c not in "0123456789abcdef" for c in identity):
        raise ValueError("invalid object digest")
    return CODE / "objects" / identity / "agent"


def active_digest():
    target = os.readlink(CODE / "active")
    parts = Path(target).parts
    if len(parts) != 3 or parts[0] != "objects" or parts[2] != "agent":
        raise ValueError("unexpected active target")
    path = object_path(parts[1])
    if digest(read_file(path, LIMIT)) != parts[1]:
        raise ValueError("object digest mismatch")
    return parts[1]


def pointer(name, identity):
    object_path(identity)
    temporary = CODE / f".{name}.{secrets.token_hex(8)}"
    temporary.symlink_to(f"objects/{identity}/agent")
    os.replace(temporary, CODE / name)
    sync_dir(CODE)
