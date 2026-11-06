"""GitHub state and publication regressions using real threshold signatures."""

from datetime import timedelta
import os
from pathlib import Path
from unittest.mock import patch

from cryptography.hazmat.primitives import serialization
from securesystemslib.signer import CryptoSigner

from . import github, github_online, github_state as state, metadata, repository
from .io import create, digest, directory, read
from .test_release import _CeremonyFixture

TARGET = "x86_64-unknown-linux-musl"
SCOPE = "stable/" + TARGET
BASE = "https://xinian-dada.github.io/NodeLite"


class GitHubPublishing(_CeremonyFixture):
    def setUp(self):
        super().setUp()
        self.publish()
        self.entry = {"channel": "stable", "target": TARGET, "root_sha256": digest(read(self.root))}
        self.config = {"schema": 1, "repositories": [self.entry], "enrollments": []}
        self.approvals = directory(self.path / "approvals")
        self.previous = directory(self.path / "previous")
        self.cache = directory(self.path / "cache")
        folder = state.folders(self.approvals, self.entry)
        generation = repository.current(self.path / "mirror")[0]
        for path in (generation / "metadata").iterdir():
            if ".root." in path.name or ".targets." in path.name:
                create(folder / path.name, read(path))
        self.candidate = self.path / "candidate"

    def signer(self, role):
        key = serialization.load_pem_private_key(read(self.roles[role][0][0]), self.secret)
        return CryptoSigner(key)

    def prepare(self, initialize=True, out=None):
        state.prepare(self.config, self.approvals, self.previous, out or self.candidate, initialize=initialize)

    def renew(self, candidate=None):
        for role in ("snapshot", "timestamp"):
            github_online.renew(self.config, candidate or self.candidate, role, self.signer(role))

    def render(self, name="public", candidate=None):
        with patch.object(github, "download", return_value=self.artifact) as download:
            result = github.render(self.config, self.previous, candidate or self.candidate, self.path / name,
                                   self.cache, "XiNian-dada/NodeLite", BASE, initialize=True)
        return result, download

    def test_initialization_static_export_and_durable_renewal(self):
        with self.assertRaisesRegex(ValueError, "explicit initialization"):
            self.prepare(initialize=False)
        self.prepare(out=self.path / "initialized")
        self.candidate = self.path / "initialized"
        self.renew()
        result, download = self.render()
        download.assert_called_once_with("XiNian-dada/NodeLite", "v3.0.5", "nodelite-agent-" + TARGET, self.cache)
        public = self.path / "public"
        self.assertEqual(result["repositories"][0]["version"], "3.0.5")
        self.assertFalse(any(path.is_symlink() for path in public.rglob("*")))
        self.assertTrue((public / SCOPE / "targets" / (digest(read(self.artifact)) + ".nodelite-agent-stable-" + TARGET)).is_file())
        state.save(self.config, public, self.previous)
        renewed = self.path / "renewed"
        self.prepare(initialize=False, out=renewed)
        self.renew(renewed)
        self.render("public-2", renewed)
        before = state.inventory(public / SCOPE / "metadata")
        after = state.inspect(self.path / "public-2" / SCOPE / "metadata", self.entry, fresh=True)
        self.assertEqual([doc.signed.version for doc in after[2]], [1, 2, 2])
        for name, raw in before.items():
            if name != "timestamp.json":
                self.assertEqual(after[0][name], raw)

    def test_expired_online_roles_can_be_renewed_without_offline_keys(self):
        self.prepare()
        self.renew()
        self.render()
        state.save(self.config, self.path / "public", self.previous)
        future = metadata.now() + timedelta(days=8)
        with patch.object(metadata, "now", return_value=future), patch.object(repository, "now", return_value=future):
            with self.assertRaises(ValueError):
                state.inspect(self.previous / SCOPE / "metadata", self.entry, fresh=True)
            renewed = self.path / "renewed"
            self.prepare(initialize=False, out=renewed)
            self.renew(renewed)
            self.render("public-2", renewed)

    def test_expired_targets_require_new_offline_approval(self):
        with patch.object(metadata, "now", return_value=metadata.now() + timedelta(days=91)):
            with self.assertRaisesRegex(ValueError, "offline renewal"):
                self.prepare()

    def test_wrong_root_scope_and_single_target_signature_are_rejected(self):
        folder = self.approvals / SCOPE / "metadata"
        self.entry["root_sha256"] = "0" * 64
        with self.assertRaises(ValueError):
            state.inspect(folder, self.entry, online=False)
        self.entry["root_sha256"] = digest(read(self.root))
        self.entry["channel"] = "testing"
        with self.assertRaises(ValueError):
            state.inspect(folder, self.entry, online=False)
        self.entry["channel"] = "stable"
        target = metadata.load(folder / "1.targets.json")
        target.signatures.pop(next(iter(target.signatures)))
        (folder / "1.targets.json").write_bytes(target.to_bytes())
        with self.assertRaises(ValueError):
            state.inspect(folder, self.entry, online=False)

    def test_online_signer_cannot_use_offline_or_other_online_role_key(self):
        self.prepare()
        for role in ("root", "targets", "timestamp"):
            with self.assertRaises(ValueError):
                github_online.renew(self.config, self.candidate, "snapshot", self.signer(role))
        with self.assertRaises(ValueError):
            github_online.renew(self.config, self.candidate, "targets", self.signer("targets"))

    def test_secret_bridge_removes_credentials_and_rejects_wrong_password(self):
        environment = {"NODELITE_ONLINE_PRIVATE_KEY": read(self.roles["snapshot"][0][0]).decode(),
                       "NODELITE_ONLINE_PASSWORD": self.secret.decode()}
        with patch.dict(os.environ, environment):
            signer = github_online.signer_from_environment()
            self.assertEqual(signer.public_key.keyid, self.signer("snapshot").public_key.keyid)
            self.assertNotIn("NODELITE_ONLINE_PRIVATE_KEY", os.environ)
            self.assertNotIn("NODELITE_ONLINE_PASSWORD", os.environ)
        environment["NODELITE_ONLINE_PASSWORD"] = "incorrect-secret-password"
        with patch.dict(os.environ, environment), self.assertRaises(ValueError):
            github_online.signer_from_environment()

    def test_tampered_release_is_not_published(self):
        self.prepare()
        self.renew()
        self.artifact.write_bytes(b"\x7fELFtampered")
        with self.assertRaises(Exception):
            self.render()

    def test_historical_metadata_removal_and_timestamp_rollback_are_rejected(self):
        self.prepare()
        self.renew()
        self.render()
        state.save(self.config, self.path / "public", self.previous)
        second = self.path / "second"
        self.prepare(initialize=False, out=second)
        self.renew(second)
        self.render("public-2", second)
        state.save(self.config, self.path / "public-2", self.previous)
        with self.assertRaises(ValueError):
            state.verify_transition(self.previous / SCOPE / "metadata", self.candidate / SCOPE / "metadata", self.entry)
        (second / SCOPE / "metadata/1.snapshot.json").unlink()
        with self.assertRaisesRegex(ValueError, "removed or changed"):
            state.verify_transition(self.previous / SCOPE / "metadata", second / SCOPE / "metadata", self.entry)

    def test_private_file_symlink_and_metadata_name_equivocation_are_rejected(self):
        folder = self.approvals / SCOPE / "metadata"
        secret = folder / "private.pem"
        create(secret, b"must not publish")
        with self.assertRaises(ValueError):
            state.inventory(folder)
        secret.unlink()
        target = folder / "1.targets.json"
        raw = read(target)
        target.unlink()
        target.symlink_to(self.artifact)
        with self.assertRaises(OSError):
            state.inventory(folder)
        target.unlink()
        create(folder / "2.targets.json", raw)
        with self.assertRaisesRegex(ValueError, "signed version differ"):
            state.inventory(folder)

    def test_offline_rollback_same_sequence_change_and_missing_state_rejected(self):
        self.prepare()
        self.renew()
        self.render()
        state.save(self.config, self.path / "public", self.previous)
        folder = self.approvals / SCOPE / "metadata"
        target = metadata.load(folder / "1.targets.json")
        target.signed.version = 2
        info = next(iter(target.signed.targets.values()))
        info.hashes["sha256"] = "0" * 64
        target.signatures.clear()
        for private, _ in self.roles["targets"]:
            target.sign(CryptoSigner(serialization.load_pem_private_key(read(private), self.secret)), append=True)
        create(folder / "2.targets.json", target.to_bytes())
        with self.assertRaisesRegex(ValueError, "same release sequence"):
            self.prepare(initialize=False, out=self.path / "bad")
        empty = directory(self.path / "lost-state")
        with self.assertRaisesRegex(ValueError, "missing durable state"):
            github.render(self.config, empty, self.candidate, self.path / "missing", self.cache,
                          "XiNian-dada/NodeLite", BASE)

    def test_empty_config_and_release_locator_and_host_budget_rejected(self):
        with self.assertRaises(ValueError):
            state.prepare({"repositories": []}, self.approvals, self.previous, self.candidate)
        for url in ("http://remote.invalid", "https://user@host/path", "https://host/path?query"):
            with self.assertRaises(ValueError):
                github.endpoint(url)
        with self.assertRaises(ValueError):
            github.download("XiNian-dada/NodeLite", "--help", "agent", self.cache)
        with self.assertRaises(ValueError):
            github.budget(901 * 1024 * 1024)
