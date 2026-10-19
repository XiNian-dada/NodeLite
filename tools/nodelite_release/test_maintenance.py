"""Readiness cannot bless an unpinned feed; collection archives rather than deletes trust."""

import os
import secrets
from unittest.mock import patch

from . import maintenance, repository
from .io import create, digest, directory, read
from .test_release import _CeremonyFixture


class Operations(_CeremonyFixture):
    def test_health_is_pinned_and_warns_before_expiry(self):
        self.publish()
        mirror = self.path / "mirror"
        pin = digest(read(self.root))
        self.assertTrue(maintenance.check(mirror, pin)["ready"])
        with self.assertRaises(ValueError):
            maintenance.check(mirror, secrets.token_hex(32))
        from datetime import timedelta
        from .metadata import now
        with patch.object(maintenance, "now", return_value=now() + timedelta(hours=7)):
            result = maintenance.check(mirror, pin)
        self.assertFalse(result["ready"])
        self.assertIn("timestamp:renew_soon", result["warnings"])
        with patch.object(maintenance, "now", return_value=now() + timedelta(hours=13)):
            self.assertIn("timestamp:expired", maintenance.check(mirror, pin)["warnings"])

    def archive_fixture(self):
        for version in range(1, 5):
            self.publish(version, version)
        mirror, archive = self.path / "mirror", directory(self.path / "archive")
        for item in (mirror / "generations").iterdir():
            os.utime(item, (1, 1))
        return mirror, archive

    def test_collect_is_dry_by_default_and_preserves_current_history(self):
        mirror, archive = self.archive_fixture()
        pointer = os.readlink(mirror / "current")
        plan = maintenance.collect(mirror, archive, keep=2)
        self.assertEqual(len(plan["retired"]), 2)
        self.assertEqual(len(list(archive.iterdir())), 0)
        result = maintenance.collect(mirror, archive, keep=2, apply=True)
        self.assertFalse(result["deleted"])
        self.assertEqual(len(list(archive.iterdir())), 2)
        self.assertEqual(len(list((mirror / "generations").iterdir())), 2)
        self.assertEqual(os.readlink(mirror / "current"), pointer)
        live = repository.current(mirror)[0]
        self.assertTrue((live / "metadata/1.targets.json").is_file())
        self.assertTrue(maintenance.check(mirror, digest(read(self.root)))["ready"])

    def test_recent_generations_and_bad_archive_are_not_moved(self):
        mirror, archive = self.archive_fixture()
        for item in (mirror / "generations").iterdir():
            os.utime(item, None)
        self.assertEqual(maintenance.collect(mirror, archive, keep=2, apply=True)["retired"], [])
        for keep, grace in ((1, 3600), (64, 3600), (2, 0)):
            with self.assertRaises(ValueError):
                maintenance.collect(mirror, archive, keep, grace, True)
        inside = directory(mirror / "private-archive")
        with self.assertRaises(ValueError):
            maintenance.collect(mirror, inside, apply=True)
        archive.chmod(0o755)
        with self.assertRaises(ValueError):
            maintenance.collect(mirror, archive, apply=True)

    def test_collection_rejects_links_unknown_files_and_incomplete_generations(self):
        mirror, archive = self.archive_fixture()
        live = repository.current(mirror)[0]
        victim = next(item for item in (mirror / "generations").iterdir() if item != live)
        victim.chmod(0o755)
        with self.assertRaises(ValueError):
            maintenance.collect(mirror, archive, keep=2, apply=True)
        victim.chmod(0o555)
        (victim / "metadata").chmod(0o755)
        outside = self.path / "outside"
        create(outside, b"keep")
        (victim / "metadata/999.targets.json").symlink_to(outside)
        (victim / "metadata").chmod(0o555)
        with self.assertRaises((OSError, ValueError)):
            maintenance.collect(mirror, archive, keep=2, apply=True)
        self.assertEqual(read(outside), b"keep")
        self.assertEqual(len(list(archive.iterdir())), 0)

    def test_maintenance_and_publication_share_an_exclusive_lock(self):
        self.publish()
        mirror = self.path / "mirror"
        archive = directory(self.path / "archive")
        with repository.locked(mirror):
            with self.assertRaises(BlockingIOError):
                maintenance.check(mirror, digest(read(self.root)))
            with self.assertRaises(BlockingIOError):
                maintenance.collect(mirror, archive)

    def test_archive_entry_substitution_cannot_chmod_a_private_file(self):
        mirror, archive = self.archive_fixture()
        victim = self.path / "private-file"
        create(victim, b"private test fixture", 0o600)
        rename = os.rename
        held = archive / "moved-generation"

        def substitute(source, target, *, src_dir_fd, dst_dir_fd):
            rename(source, target, src_dir_fd=src_dir_fd, dst_dir_fd=dst_dir_fd)
            rename(target, held.name, src_dir_fd=dst_dir_fd, dst_dir_fd=dst_dir_fd)
            os.symlink(victim, target, dir_fd=dst_dir_fd)

        with patch.object(maintenance.os, "rename", side_effect=substitute):
            with self.assertRaisesRegex(ValueError, "archive entry changed"):
                maintenance.collect(mirror, archive, keep=2, apply=True)
        self.assertEqual(victim.stat().st_mode & 0o777, 0o600)
        self.assertEqual(held.stat().st_mode & 0o777, 0o555)
        self.assertTrue(maintenance.check(mirror, digest(read(self.root)))["ready"])

    def test_archive_path_substitution_does_not_redirect_the_open_directory(self):
        mirror, archive = self.archive_fixture()
        original = self.path / "original-archive"
        rename = os.rename
        substituted = False

        def substitute(source, target, *, src_dir_fd, dst_dir_fd):
            nonlocal substituted
            if not substituted:
                rename(archive, original)
                directory(archive)
                substituted = True
            rename(source, target, src_dir_fd=src_dir_fd, dst_dir_fd=dst_dir_fd)

        with patch.object(maintenance.os, "rename", side_effect=substitute):
            maintenance.collect(mirror, archive, keep=2, apply=True)
        self.assertEqual(len(list(archive.iterdir())), 0)
        self.assertEqual(len(list(original.iterdir())), 2)
        self.assertTrue(all(item.stat().st_mode & 0o777 == 0o555 for item in original.iterdir()))


if __name__ == "__main__":
    import unittest
    unittest.main()
