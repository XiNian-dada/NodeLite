"""Fingerprint and path regressions without privileged commands or host installation."""

from pathlib import Path
import secrets
import tempfile
import unittest
from unittest.mock import patch

from tools.nodelite_release.io import create, digest, directory, json_bytes

from . import bundle, installer
from .common import UNITS


class Bundles(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name)
        directory(self.path / "systemd")
        self.files = {"root.json": b"{}", "nodelite-agent": b"\x7fELFtest-agent", "nodelite-updater": b"\x7fELFtest-updater"}
        self.files.update({"systemd/" + name: b"[Unit]\n" for name in UNITS})
        for name, data in self.files.items():
            create(self.path / name, data)
        self.manifest = {"schema": 1, "channel": "stable", "target": "x86_64-unknown-linux-musl",
                         "baseline_sequence": 1, "minimum_sequence": 1,
                         "files": {name: digest(data) for name, data in self.files.items()}}
        self.root_hash = digest(self.files["root.json"])
        self.refresh()

    def refresh(self):
        path = self.path / "manifest.json"
        path.unlink(missing_ok=True)
        data = json_bytes(self.manifest)
        create(path, data)
        self.bundle_hash = digest(data)

    def test_complete_snapshot_requires_both_independent_fingerprints(self):
        manifest, files = bundle.inspect(self.path, self.bundle_hash, self.root_hash)
        self.assertEqual(manifest, self.manifest)
        self.assertEqual(files, self.files)
        for first, second in ((secrets.token_hex(32), self.root_hash), (self.bundle_hash, secrets.token_hex(32))):
            with self.assertRaises(ValueError):
                bundle.inspect(self.path, first, second)

    def test_tamper_and_link_rejected_before_any_execution(self):
        agent = self.path / "nodelite-agent"
        agent.unlink()
        create(agent, b"\x7fELFchanged")
        with self.assertRaises(ValueError):
            bundle.inspect(self.path, self.bundle_hash, self.root_hash)
        agent.unlink()
        agent.symlink_to(self.path / "nodelite-updater")
        with self.assertRaises(OSError):
            bundle.inspect(self.path, self.bundle_hash, self.root_hash)

    def test_path_escape_private_key_and_extra_field_rejected(self):
        for name in ("../../outside", "root-private.pem", "install.sh"):
            self.manifest["files"][name] = secrets.token_hex(32)
            self.refresh()
            with self.assertRaises(ValueError):
                bundle.inspect(self.path, self.bundle_hash, self.root_hash)
            del self.manifest["files"][name]
        self.manifest["command"] = "not executable input"
        self.refresh()
        with self.assertRaises(ValueError):
            bundle.inspect(self.path, self.bundle_hash, self.root_hash)

    def test_installer_descriptor_and_architecture_checked_before_migration(self):
        value = {"schema": 1, "bundle_sha256": self.bundle_hash, "root_sha256": self.root_hash,
                 "metadata_url": "https://example.invalid/metadata/", "targets_url": "https://example.invalid/targets/"}
        descriptor = self.path / "enrollment.json"
        create(descriptor, json_bytes(value))
        self.assertEqual(installer.validate(self.path, self.manifest["target"])[1], self.bundle_hash)
        with self.assertRaises(ValueError):
            installer.validate(self.path, "aarch64-unknown-linux-musl")
        descriptor.unlink()
        value["command"] = "never execute descriptor commands"
        create(descriptor, json_bytes(value))
        with self.assertRaises(ValueError):
            installer.validate(self.path, self.manifest["target"])

    def test_installer_waits_for_actual_commit_and_reports_rollback(self):
        self.assertFalse(installer.outcome(None))
        for phase in ("prepared", "stopping", "trial", "rollback_intent"):
            self.assertFalse(installer.outcome({"phase": phase, "manual_recovery": False}))
        for phase in ("stable", "committed"):
            self.assertTrue(installer.outcome({"phase": phase, "manual_recovery": False}))
        for phase, recovery in (("rolled_back", False), ("manual_recovery", False), ("committed", True)):
            with self.assertRaises(RuntimeError):
                installer.outcome({"phase": phase, "manual_recovery": recovery})
        stable = {"phase": "stable", "manual_recovery": False, "installed_sequence": 1}
        committed = {**stable, "phase": "committed", "installed_sequence": 2}
        trial = {**stable, "phase": "trial"}
        with patch.object(installer, "status", side_effect=[None, stable, trial, None, committed]), \
                patch.object(installer.time, "sleep"), patch.object(installer, "command") as command, \
                patch("tools.nodelite_migrate.audit.check") as audit:
            self.assertEqual(installer.upgrade()["installed_sequence"], 2)
            command.assert_called_once_with("/usr/bin/systemctl", "start", "nodelite-updater-apply.service", timeout=100)
            self.assertEqual(audit.call_count, 2)

    def test_lab_fault_binary_cannot_be_mistaken_for_normal_updater(self):
        path = self.path / "nodelite-updater"
        path.unlink()
        data = b"\x7fELF/etc/nlup-lab-guest"
        create(path, data)
        self.manifest["files"]["nodelite-updater"] = digest(data)
        self.refresh()
        with self.assertRaises(ValueError):
            bundle.inspect(self.path, self.bundle_hash, self.root_hash)

    def test_url_policy_cannot_smuggle_credentials_commands_or_remote_http(self):
        valid = "https://updates.example.invalid/metadata/"
        for bad in ("file:///root/", "http://example.invalid/", "https://user:secret@example.invalid/", valid + "?token=x",
                    valid + "#fragment", 'https://example.invalid/"\ncommand = "sh"', "https://example.invalid/\\"):
            with self.assertRaises(ValueError):
                bundle.policy(self.manifest, self.files, bad, valid)
        self.assertIn(b"enabled = true", bundle.policy(self.manifest, self.files, valid, valid))


if __name__ == "__main__":
    unittest.main()
