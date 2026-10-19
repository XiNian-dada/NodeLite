"""Bounded inode snapshots and durable, exclusive operator output publication."""

import hashlib
import json
import os
from pathlib import Path
import secrets
import stat

LIMIT = 32 * 1024 * 1024


def read(path, limit=2 * 1024 * 1024):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_size > limit:
            raise ValueError("unsafe input inode or size")
        data = stream.read(limit + 1)
        if len(data) > limit:
            raise ValueError("input exceeds size limit")
        return data


def digest(data):
    return hashlib.sha256(data).hexdigest()


def unique(pairs):
    result = {}
    for name, value in pairs:
        if name in result:
            raise ValueError("duplicate JSON member")
        result[name] = value
    return result


def decode(data):
    return json.loads(data, object_pairs_hook=unique)


def json_bytes(value):
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()


def sync(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def directory(path, mode=0o700):
    path = Path(path)
    path.mkdir(mode=mode)
    path.chmod(mode)
    sync(path.parent)
    return path


def owned_directory(path, private=False):
    info = Path(path).lstat()
    mode = stat.S_IMODE(info.st_mode)
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or mode & 0o022 \
            or private and mode != 0o700:
        raise ValueError("operator directory ownership or mode rejected")


def create(path, data, mode=0o644):
    path = Path(path)
    owned_directory(path.parent, private=mode == 0o600)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    with os.fdopen(fd, "wb") as stream:
        os.fchmod(stream.fileno(), mode)
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    sync(path.parent)


def atomic(path, data, mode=0o644):
    path = Path(path)
    temporary = path.with_name(".publish-" + secrets.token_hex(16))
    try:
        create(temporary, data, mode)
        os.replace(temporary, path)
        sync(path.parent)
    finally:
        temporary.unlink(missing_ok=True)
