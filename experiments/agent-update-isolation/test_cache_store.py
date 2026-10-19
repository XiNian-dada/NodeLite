"""Offline atomic-cache boundary tests; no TUF package, VM, mounts or SSH required."""

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from cache_store import CacheBusy, CacheInvalid, Store
from lab_io import digest


class StoreTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.store = Store(Path(temporary.name) / "cache", b"public-root")
        self.store.provision(1000)

    def generation(self):
        state = self.store.load()
        directory = self.store.stage(state)
        (directory / "metadata/root.json").write_bytes(b"root")
        return state, directory

    def test_old_artifact_survives_a_higher_metadata_floor(self):
        state, directory = self.generation()
        state = self.store.publish_metadata(state, directory, sequence=1, hashes={"sha256": digest(b"first")}, now=1000)
        self.store.publish_artifact(state, b"old artifact", 1, lambda _: None)
        state, directory = self.generation()
        self.store.publish_metadata(state, directory, sequence=2, hashes={"sha256": digest(b"second")}, now=1001)
        state = self.store.load()
        self.assertEqual(state["sequence"], 2)
        self.assertEqual(self.store.artifact(state).read_bytes(), b"old artifact")

    def test_missing_or_corrupt_journal_never_reinitializes(self):
        for data in (b"", b"{broken", b'{}'):
            with self.subTest(data=data):
                self.store.state_path.write_bytes(data)
                with self.assertRaises(CacheInvalid):
                    self.store.load()
        self.store.state_path.unlink()
        with self.assertRaises(CacheInvalid):
            self.store.load()

    def test_bootstrap_anchor_cannot_change(self):
        with self.assertRaises(CacheInvalid):
            Store(self.store.directory, b"different-root").load()

    def test_metadata_corruption_is_not_silently_discarded(self):
        state, directory = self.generation()
        self.store.publish_metadata(state, directory)
        (directory / "metadata/root.json").write_bytes(b"changed")
        with self.assertRaises(CacheInvalid):
            self.store.load()

    def test_unexpected_symlink_is_rejected(self):
        state, directory = self.generation()
        (directory / "metadata/timestamp.json").symlink_to("/etc/passwd")
        with self.assertRaises(CacheInvalid):
            self.store.publish_metadata(state, directory)

    def test_second_transaction_cannot_enter(self):
        with self.store.lock(), self.assertRaises(CacheBusy), self.store.lock():
            self.fail("second lock was acquired")

    def test_failed_generation_sync_keeps_the_previous_journal(self):
        before = self.store.state_path.read_bytes()
        state, directory = self.generation()
        with patch("cache_store.sync_metadata", side_effect=OSError("storage failure")), self.assertRaises(OSError):
            self.store.publish_metadata(state, directory)
        self.assertEqual(self.store.state_path.read_bytes(), before)

    def test_corrupt_committed_object_requires_recovery(self):
        state, directory = self.generation()
        state = self.store.publish_metadata(state, directory, sequence=1, hashes={"sha256": digest(b"payload")}, now=1000)
        path = self.store.publish_artifact(state, b"payload", 1, lambda _: None)
        path.write_bytes(b"wrong")
        with self.assertRaises(CacheInvalid):
            self.store.load()

    def test_invalid_high_water_digest_requires_recovery(self):
        state, directory = self.generation()
        self.store.publish_metadata(state, directory, sequence=1, hashes={"sha256": "short"}, now=1000)
        with self.assertRaises(CacheInvalid):
            self.store.load()

    def test_active_metadata_directory_cannot_be_a_symlink(self):
        state, directory = self.generation()
        self.store.publish_metadata(state, directory)
        (directory / "metadata").rename(directory / "saved")
        (directory / "metadata").symlink_to("saved", target_is_directory=True)
        with self.assertRaises(CacheInvalid):
            self.store.load()


if __name__ == "__main__":
    unittest.main()
