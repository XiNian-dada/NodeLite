"""Bounded ext4 exhaustion in regular-file loop volumes, marked guest only."""

from contextlib import contextmanager
import errno
import os
from pathlib import Path
import tempfile

from resilience_guest import ROOT
from setup_guest import guard, run


def validate_mount(mountpoint, image):
    source = run("findmnt", "-n", "-o", "SOURCE", "--mountpoint", str(mountpoint)).stdout.strip()
    if not source.startswith("/dev/loop") or "[" in source:
        raise RuntimeError("refusing to unmount an unexpected test filesystem")
    backing = run("losetup", "-n", "-O", "BACK-FILE", source).stdout.strip()
    if Path(backing).resolve() != image.resolve():
        raise RuntimeError("test loop backing file changed")


@contextmanager
def volume():
    guard()
    directory = Path(tempfile.mkdtemp(prefix="volume-", dir=ROOT))
    directory.chmod(0o755)
    image, mountpoint = directory / "ext4.img", directory / "mount"
    fd = os.open(image, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        os.ftruncate(fd, 64 * 1024 * 1024)
    finally:
        os.close(fd)
    mountpoint.mkdir(mode=0o755)
    run("mkfs.ext4", "-q", "-F", "-m", "0", "-N", "128", str(image))
    run("mount", "-o", "loop,nosuid,nodev,noexec", str(image), str(mountpoint))
    try:
        validate_mount(mountpoint, image)
        yield mountpoint
    finally:
        validate_mount(mountpoint, image)
        run("umount", str(mountpoint))


def capacity(path):
    value = os.statvfs(path)
    return {"free_bytes": value.f_bavail * value.f_frsize, "free_inodes": value.f_ffree}


def exhaust(mountpoint, kind, leave_bytes=0):
    filler = mountpoint / "filler"
    filler.mkdir(mode=0o700)
    paths, written = [], 0
    try:
        if kind == "disk":
            path = filler / "blocks"
            paths.append(path)
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            try:
                while written <= 64 * 1024 * 1024:
                    written += os.write(fd, b"F" * 65536)
                raise RuntimeError("test volume exceeded the configured size")
            finally:
                os.close(fd)
        else:
            for index in range(1024):
                path = filler / f"inode-{index}"
                fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                os.close(fd)
                paths.append(path)
            raise RuntimeError("test volume exceeded the configured inode bound")
    except OSError as error:
        if error.errno != errno.ENOSPC:
            raise
    full = capacity(mountpoint)
    assert full["free_bytes" if kind == "disk" else "free_inodes"] == 0, full
    if leave_bytes:
        os.truncate(paths[0], max(0, written - leave_bytes))
    os.sync()
    return {"errno": errno.ENOSPC, "full_capacity": full, "attempt_capacity": capacity(mountpoint)}, paths


def release(paths):
    for path in paths:
        path.unlink()
    os.sync()
