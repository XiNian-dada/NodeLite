"""Fingerprint and path regressions without privileged commands or host installation."""

from pathlib import Path
import io
import secrets
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from tools.nodelite_release.io import create, digest, directory, json_bytes

from tools.nodelite_migrate import bundle, installer
from tools.nodelite_migrate.common import UNITS


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

    def test_installer_resume_waits_for_legacy_restart_without_resetting_trust(self):
        create(self.path / "journal.json", b"{}")
        with patch.object(installer, "MIGRATION", self.path), \
                patch.object(installer, "validate", return_value=("verified",)) as validate, \
                patch.object(installer.transaction, "recover") as recover, \
                patch.object(installer.transaction, "resume", return_value={"migrated": True}) as resume, \
                patch.object(installer.transaction, "apply") as apply, \
                patch.object(installer, "systemctl") as systemctl:
            self.assertTrue(installer.enroll(self.path, "fixed-target")["migrated"])
            validate.assert_called_once_with(self.path, "fixed-target")
            recover.assert_called_once_with()
            systemctl.assert_called_once_with("start", "nodelite-agent.service")
            resume.assert_called_once_with("verified")
            apply.assert_not_called()

    def test_rejected_preflight_never_reaches_stop_or_switch(self):
        from tools.nodelite_migrate import transaction
        from tools.nodelite_migrate.common import PreflightError

        with patch.object(transaction, "MIGRATION", self.path), \
                patch.object(transaction, "accounts"), patch.object(transaction, "ensure_layout"), \
                patch.object(transaction, "stage_programs"), patch.object(transaction, "create"), \
                patch.object(transaction, "command"), \
                patch.object(transaction, "systemctl", return_value=subprocess.CompletedProcess([], 1)) as systemctl, \
                patch.object(transaction, "stop_barrier") as stop, \
                patch.object(transaction, "config_barrier") as credentials, \
                patch.object(transaction, "atomic") as switch, patch.object(transaction, "recover") as recover:
            with self.assertRaisesRegex(PreflightError, "before stopping.*1..10"):
                transaction.handoff({}, self.files, b"policy", 60)
            systemctl.assert_called_with("restart", "nodelite-agent-preflight.service", check=False)
            stop.assert_not_called()
            credentials.assert_not_called()
            switch.assert_not_called()
            recover.assert_called_once_with()

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

    def test_bootstrap_extractor_rejects_links_duplicates_traversal_and_oversize(self):
        helper = Path(__file__).resolve().parents[2] / "scripts/install-agent-signed.sh"
        script = helper.read_text().split('<<\'PY\'\n', 1)[1].split('\nPY\n', 1)[0]
        for name, kind, size in (("../../escape", tarfile.REGTYPE, 0),
                                 ("root.json", tarfile.SYMTYPE, 0),
                                 ("root.json", tarfile.LNKTYPE, 0),
                                 ("root.json", tarfile.REGTYPE, 262145),
                                 ("tools/nodelite_release/keys.py", tarfile.REGTYPE, 0)):
            with tempfile.TemporaryDirectory() as temporary:
                archive_path = Path(temporary) / "package.tar.gz"
                with tarfile.open(archive_path, "w:gz") as archive:
                    entry = tarfile.TarInfo(name)
                    entry.type, entry.size, entry.linkname = kind, size, "outside"
                    archive.addfile(entry, io.BytesIO(b"x" * size))
                result = subprocess.run([sys.executable, "-I", "-c", script, temporary], capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(list(Path(temporary).iterdir()), [archive_path])
        with tempfile.TemporaryDirectory() as temporary:
            with tarfile.open(Path(temporary) / "package.tar.gz", "w:gz") as archive:
                for _ in range(2):
                    archive.addfile(tarfile.TarInfo("root.json"), io.BytesIO())
            result = subprocess.run([sys.executable, "-I", "-c", script, temporary], capture_output=True)
            self.assertNotEqual(result.returncode, 0)

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
