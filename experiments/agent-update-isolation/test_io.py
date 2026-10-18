"""Unprivileged, offline regression checks for hostile inputs and durable replacement."""

import errno
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from lab_io import atomic_bytes, bounded_at, decode


class FileBoundaryTests(unittest.TestCase):
    def setUp(self):
        parent = Path(__file__).resolve().parent / ".io-tests"
        parent.mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="nlup-io-test-", dir=parent)
        self.root = Path(self.temporary.name)
        self.directory = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)

    def tearDown(self):
        os.close(self.directory)
        self.temporary.cleanup()

    def test_private_snapshot_is_not_aliased_to_attacker_fd(self):
        source = self.root / "source"
        source.write_bytes(b"validated fixture")
        with source.open("r+b") as attacker:
            snapshot = bounded_at(self.directory, "source", 100)
            atomic_bytes(self.root / "private", snapshot)
            attacker.write(b"changed malicious")
        self.assertEqual((self.root / "private").read_bytes(), b"validated fixture")

    def test_symlink_rejected(self):
        (self.root / "input").symlink_to("/etc/passwd")
        with self.assertRaises(OSError):
            bounded_at(self.directory, "input", 100)

    def test_hardlink_rejected(self):
        (self.root / "input").write_bytes(b"fixture")
        os.link(self.root / "input", self.root / "alias")
        with self.assertRaises(ValueError):
            bounded_at(self.directory, "input", 100)

    def test_fifo_does_not_block(self):
        os.mkfifo(self.root / "input")
        with self.assertRaises(ValueError):
            bounded_at(self.directory, "input", 100)

    def test_oversize_rejected(self):
        (self.root / "input").write_bytes(b"abcd")
        with self.assertRaises(ValueError):
            bounded_at(self.directory, "input", 3)

    def test_duplicate_json_fields_rejected(self):
        with self.assertRaises(ValueError):
            decode(b'{"sequence":1,"sequence":2}')

    def test_explicit_object_mode_survives_private_umask(self):
        previous = os.umask(0o077)
        try:
            atomic_bytes(self.root / "agent", b"fixture", 0o550)
        finally:
            os.umask(previous)
        self.assertEqual((self.root / "agent").stat().st_mode & 0o777, 0o550)

    def test_failed_file_fsync_keeps_original_and_cleans_temporary(self):
        path = self.root / "journal"
        atomic_bytes(path, b"old")
        with patch("lab_io.os.fsync", side_effect=OSError(errno.ENOSPC, "injected full disk")):
            with self.assertRaises(OSError):
                atomic_bytes(path, b"new")
        self.assertEqual(path.read_bytes(), b"old")
        self.assertEqual(list(self.root.iterdir()), [path])

    def test_failed_directory_fsync_is_reported_not_hidden(self):
        path = self.root / "journal"
        atomic_bytes(path, b"old")
        with patch("lab_io.sync_dir", side_effect=OSError(errno.EIO, "injected sync failure")):
            with self.assertRaises(OSError):
                atomic_bytes(path, b"new")
        self.assertEqual(path.read_bytes(), b"new")


if __name__ == "__main__":
    unittest.main()
