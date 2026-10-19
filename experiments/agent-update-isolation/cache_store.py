"""Experimental durable cache generations; old artifacts and credential files are separate."""

from contextlib import contextmanager
import fcntl
import math
import os
from pathlib import Path
import re
import secrets
import shutil
import stat

from lab_io import LIMIT, atomic_bytes, atomic_json, decode, digest, read_file, sync_dir


class CacheInvalid(ValueError):
    pass


class CacheBusy(RuntimeError):
    pass


def metadata_inventory(directory):
    result = {}
    for path in sorted(directory.rglob("*")):
        relative = str(path.relative_to(directory))
        if path.is_symlink():
            target = os.readlink(path)
            if relative != "root.json" or not re.fullmatch(r"root_history/[1-9][0-9]*\.root\.json", target):
                raise CacheInvalid("unexpected cache symlink")
            result[relative] = {"link": target}
        elif path.is_dir():
            if relative != "root_history":
                raise CacheInvalid("unexpected cache directory")
        else:
            if not re.fullmatch(r"(?:root_history/[1-9][0-9]*\.root|root|timestamp|snapshot|targets)\.json", relative):
                raise CacheInvalid("unexpected cache filename")
            result[relative] = {"sha256": digest(read_file(path, 2 * 1024 * 1024))}
        if len(result) > 64:
            raise CacheInvalid("cache metadata limit exceeded")
    if "root.json" in result and "link" in result["root.json"] \
            and result["root.json"]["link"] not in result:
        raise CacheInvalid("cache root history missing")
    return result


def sync_metadata(directory):
    metadata_inventory(directory)
    for path in directory.rglob("*"):
        if not path.is_symlink() and path.is_file():
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
    if (directory / "root_history").exists():
        sync_dir(directory / "root_history")
    sync_dir(directory)
    sync_dir(directory.parent)
    sync_dir(directory.parent.parent)


class Store:
    def __init__(self, directory, bootstrap):
        self.directory, self.anchor = directory, digest(bootstrap)
        self.state_path = directory / "state.json"

    def provision(self, now):
        self.directory.mkdir(parents=True, mode=0o700, exist_ok=True)
        if any(self.directory.iterdir()):
            raise CacheInvalid("refusing to reprovision an existing cache")
        self.directory.chmod(0o700)
        for name in ("generations", "objects"):
            (self.directory / name).mkdir(mode=0o700)
        self.save({"schema": 1, "anchor": self.anchor, "active": None, "metadata": {},
                   "sequence": 0, "hashes": {}, "time_high": now, "artifact": None})
        sync_dir(self.directory.parent)

    def save(self, state):
        atomic_json(self.state_path, state)

    @contextmanager
    def lock(self):
        fd = os.open(self.directory / "lock", os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        try:
            if not stat.S_ISREG(os.fstat(fd).st_mode) or os.fstat(fd).st_nlink != 1:
                raise CacheInvalid("unsafe cache lock")
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise CacheBusy("cache transaction already running") from error
            yield
        finally:
            os.close(fd)

    def load(self):
        try:
            if any((self.directory / name).is_symlink() or not (self.directory / name).is_dir()
                   for name in ("generations", "objects")):
                raise CacheInvalid("unsafe cache directories")
            state = decode(read_file(self.state_path, 65536))
            if set(state) != {"schema", "anchor", "active", "metadata", "sequence", "hashes", "time_high", "artifact"} \
                    or type(state["schema"]) is not int or state["schema"] != 1 or state["anchor"] != self.anchor \
                    or type(state["sequence"]) is not int or not 0 <= state["sequence"] < 2**53 \
                    or type(state["time_high"]) not in (int, float) or not math.isfinite(state["time_high"]) \
                    or state["time_high"] <= 0 or not isinstance(state["metadata"], dict) \
                    or not isinstance(state["hashes"], dict):
                raise CacheInvalid("cache journal schema rejected")
            if state["active"] is not None:
                directory = self.generation(state["active"]) / "metadata"
                if directory.is_symlink() or not directory.is_dir() or metadata_inventory(directory) != state["metadata"]:
                    raise CacheInvalid("committed metadata cache changed")
            elif state["metadata"] or state["sequence"] or state["hashes"] or state["artifact"]:
                raise CacheInvalid("cache generation missing")
            lengths = {"sha256": 64, "sha512": 128}
            if state["sequence"] and ("sha256" not in state["hashes"] or any(
                    algorithm not in lengths or not isinstance(value, str)
                    or not re.fullmatch(r"[0-9a-f]{" + str(lengths[algorithm]) + "}", value)
                    for algorithm, value in state["hashes"].items())):
                raise CacheInvalid("cache release digest rejected")
            if not state["sequence"] and state["hashes"]:
                raise CacheInvalid("unbound release digest")
            self.artifact(state)
            return state
        except (OSError, ValueError, KeyError, TypeError) as error:
            raise CacheInvalid("cache state requires explicit recovery") from error

    def generation(self, identity):
        if not isinstance(identity, str) or not re.fullmatch(r"[0-9a-f]{32}", identity):
            raise CacheInvalid("invalid cache generation")
        path = self.directory / "generations" / identity
        if path.is_symlink() or (path.exists() and not path.is_dir()):
            raise CacheInvalid("unsafe cache generation")
        return path

    def stage(self, state):
        directory = self.generation(secrets.token_hex(16))
        directory.mkdir(mode=0o700)
        if state["active"]:
            shutil.copytree(self.generation(state["active"]) / "metadata", directory / "metadata", symlinks=True)
        else:
            (directory / "metadata").mkdir(mode=0o700)
        (directory / "downloads").mkdir(mode=0o700)
        return directory

    def publish_metadata(self, state, directory, *, sequence=None, hashes=None, now=None):
        sync_metadata(directory / "metadata")
        updated = {**state, "active": directory.name,
                   "metadata": metadata_inventory(directory / "metadata")}
        if sequence is not None:
            updated.update(sequence=sequence, hashes=hashes, time_high=max(state["time_high"], now))
        self.save(updated)
        return updated

    def object_path(self, identity):
        if not isinstance(identity, str) or not re.fullmatch(r"[0-9a-f]{64}", identity):
            raise CacheInvalid("invalid artifact digest")
        return self.directory / "objects" / identity

    def artifact(self, state):
        item = state["artifact"]
        if item is None:
            return None
        if set(item) != {"sha256", "size", "sequence"} or type(item["size"]) is not int \
                or not 0 < item["size"] <= LIMIT or type(item["sequence"]) is not int \
                or not 0 < item["sequence"] <= state["sequence"]:
            raise CacheInvalid("invalid artifact journal")
        path = self.object_path(item["sha256"])
        payload = read_file(path, LIMIT)
        if len(payload) != item["size"] or digest(payload) != item["sha256"]:
            raise CacheInvalid("committed artifact changed")
        return path

    def publish_artifact(self, state, payload, sequence, checkpoint):
        identity = digest(payload)
        path = self.object_path(identity)
        atomic_bytes(path, payload)
        checkpoint("object_durable")
        self.save({**state, "artifact": {"sha256": identity, "size": len(payload), "sequence": sequence}})
        checkpoint("artifact_committed")
        return path
