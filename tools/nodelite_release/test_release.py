"""Real encrypted keys, detached thresholds and immutable public repository regressions."""

import os
from pathlib import Path
import secrets
import tempfile
import unittest
from unittest.mock import patch

from cryptography.hazmat.primitives import serialization
from . import keys, metadata, repository
from .io import create, decode, directory, json_bytes, read


class Ceremony(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name)
        self.secret = secrets.token_urlsafe(24).encode()
        self.roles = {}
        for role, count in (("root", 3), ("targets", 2), ("snapshot", 1), ("timestamp", 1)):
            self.roles[role] = []
            for index in range(count):
                name = f"{role}-{index}"
                private, public = self.path / (name + ".pem"), self.path / (name + ".pub.json")
                keys.generate(private, public, self.secret)
                self.roles[role].append((private, public))
        self.policy = self.path / "policy.json"
        self.write_policy()
        self.roots = directory(self.path / "roots")
        self.root_unsigned = self.path / "root.unsigned"
        metadata.root_draft(self.policy, self.root_unsigned)
        self.root = self.roots / "1.root.json"
        self.assemble(self.root_unsigned, self.roles["root"][:2], self.root)
        self.artifact = self.path / "agent"
        create(self.artifact, b"\x7fELF" + secrets.token_bytes(64))

    def write_policy(self):
        self.policy.unlink(missing_ok=True)
        create(self.policy, json_bytes({"roles": {role: {"threshold": 2 if role in {"root", "targets"} else 1,
            "keys": [str(public.name) for _, public in entries]} for role, entries in self.roles.items()}}))

    def assemble(self, unsigned, signers, output, previous=None):
        approvals = []
        for private, _ in signers:
            approval = self.path / ("approval-" + secrets.token_hex(8))
            keys.approve(unsigned, private, approval, self.secret)
            approvals.append(approval)
        metadata.assemble(unsigned, approvals, getattr(self, "root", None), output, previous)

    def release(self, sequence=1, version=1, channel="stable"):
        work = directory(self.path / ("release-" + secrets.token_hex(8)))
        unsigned, targets = work / "targets.unsigned", work / "targets.json"
        metadata.target_draft(self.artifact, channel, "x86_64-unknown-linux-musl", "3.0.5", sequence, 1, version, unsigned)
        self.assemble(unsigned, self.roles["targets"], targets)
        snapshot, timestamp = work / "snapshot.json", work / "timestamp.json"
        for role, child, output in (("snapshot", targets, snapshot), ("timestamp", snapshot, timestamp)):
            draft = work / (role + ".unsigned")
            metadata.role_draft(role, child, self.root, draft)
            self.assemble(draft, self.roles[role], output)
        return targets, snapshot, timestamp

    def publish(self, sequence=1, version=1, channel="stable"):
        paths = self.release(sequence, version, channel)
        return repository.publish(self.roots, *paths, self.artifact, self.path / "mirror")

    def test_private_keys_are_encrypted_owner_only_and_not_overwritten(self):
        private, public = self.roles["root"][0]
        self.assertIn(b"ENCRYPTED PRIVATE KEY", read(private))
        self.assertEqual(private.stat().st_mode & 0o777, 0o600)
        with self.assertRaises(TypeError):
            serialization.load_pem_private_key(read(private), password=None)
        with self.assertRaises(ValueError):
            keys.generate(private, public, self.secret)

    def test_wrong_passphrase_and_broad_private_mode_rejected(self):
        private, _ = self.roles["targets"][0]
        with self.assertRaises(ValueError):
            keys.approve(self.root_unsigned, private, self.path / "bad", secrets.token_bytes(16))
        private.chmod(0o644)
        with self.assertRaises(ValueError):
            keys.approve(self.root_unsigned, private, self.path / "bad", self.secret)

    def test_threshold_duplicate_and_payload_substitution_rejected(self):
        approval = self.path / "approval"
        keys.approve(self.root_unsigned, self.roles["root"][0][0], approval, self.secret)
        with self.assertRaises(Exception):
            metadata.assemble(self.root_unsigned, [approval], None, self.path / "bad")
        with self.assertRaises(ValueError):
            metadata.assemble(self.root_unsigned, [approval, approval], None, self.path / "bad")
        altered = metadata.load(self.root_unsigned)
        altered.signed.version += 1
        changed = self.path / "altered"
        create(changed, altered.to_bytes())
        with self.assertRaises(ValueError):
            metadata.assemble(changed, [approval], None, self.path / "bad")

    def test_cross_role_keys_and_fake_key_ids_rejected(self):
        root = metadata.load(self.root).signed
        root.roles["targets"].keyids[0] = root.roles["root"].keyids[0]
        with self.assertRaises(ValueError):
            metadata.root_safe(root)
        root = metadata.load(self.root).signed
        keyid = root.roles["root"].keyids[0]
        root.keys["a" * 64] = root.keys.pop(keyid)
        root.roles["root"].keyids[0] = "a" * 64
        with self.assertRaises(ValueError):
            metadata.root_safe(root)

    def test_atomic_publication_retains_history_and_has_no_private_keys(self):
        first = self.publish()
        old = os.readlink(self.path / "mirror/current")
        second = self.publish(2, 2)
        self.assertNotEqual(first["generation"], second["generation"])
        self.assertTrue((self.path / "mirror" / old).is_dir())
        current = repository.current(self.path / "mirror")[0]
        self.assertTrue((current / "metadata/1.targets.json").exists())
        for path in current.rglob("*"):
            self.assertFalse(path.is_symlink())
            if path.is_file():
                self.assertNotIn(b"PRIVATE KEY", read(path))

    def test_publication_downgrade_equivocation_and_channel_rejected(self):
        self.publish(2, 2)
        pointer = os.readlink(self.path / "mirror/current")
        for sequence, version, channel in ((1, 3, "stable"), (3, 2, "stable"), (3, 3, "testing")):
            with self.assertRaises(ValueError):
                self.publish(sequence, version, channel)
        self.artifact.unlink()
        create(self.artifact, b"\x7fELFchanged")
        with self.assertRaises(ValueError):
            self.publish(2, 3)
        self.assertEqual(os.readlink(self.path / "mirror/current"), pointer)

    def test_unsigned_or_corrupt_child_never_changes_current(self):
        self.publish()
        pointer = os.readlink(self.path / "mirror/current")
        paths = self.release(2, 2)
        paths[0].unlink()
        create(paths[0], b"{}")
        with self.assertRaises(Exception):
            repository.publish(self.roots, *paths, self.artifact, self.path / "mirror")
        self.assertEqual(os.readlink(self.path / "mirror/current"), pointer)

    def test_root_rotation_requires_both_old_and_new_thresholds(self):
        old_keys = self.roles["root"]
        self.roles["root"] = []
        for index in range(3):
            private, public = self.path / f"next-{index}.pem", self.path / f"next-{index}.pub.json"
            keys.generate(private, public, self.secret)
            self.roles["root"].append((private, public))
        self.write_policy()
        candidate = self.path / "next.unsigned"
        metadata.root_draft(self.policy, candidate, self.root)
        for signers in (old_keys[:1] + self.roles["root"][:2], old_keys[:2] + self.roles["root"][:1]):
            with self.assertRaises(Exception):
                self.assemble(candidate, signers, self.path / "bad", self.root)
        self.assemble(candidate, old_keys[:2] + self.roles["root"][:2], self.roots / "2.root.json", self.root)
        self.assertEqual(len(repository.roots(self.roots)), 2)

    def test_missing_pointer_is_not_silently_recreated(self):
        self.publish()
        (self.path / "mirror/current").unlink()
        with self.assertRaises(ValueError):
            self.publish(2, 2)

    def test_scope_json_and_symlink_inputs_rejected(self):
        with self.assertRaises(ValueError):
            decode(b'{"roles":{},"roles":{}}')
        link = self.path / "link"
        link.symlink_to(self.artifact)
        with self.assertRaises(OSError):
            metadata.target_draft(link, "stable", "x86_64-unknown-linux-musl", "3.0.5", 1, 1, 1, self.path / "bad")
        with self.assertRaises(ValueError):
            metadata.target_draft(self.artifact, "../stable", "x86_64-unknown-linux-musl", "3.0.5", 1, 1, 1, self.path / "bad")

    def test_cargo_aliased_artifact_is_copied_but_private_key_aliases_are_rejected(self):
        os.link(self.artifact, self.path / "artifact-alias")
        metadata.target_draft(self.artifact, "stable", "x86_64-unknown-linux-musl", "3.0.5", 1, 1, 1, self.path / "draft")
        private = self.roles["root"][0][0]
        os.link(private, self.path / "private-alias")
        with self.assertRaises(ValueError):
            keys.approve(self.root_unsigned, private, self.path / "approval", self.secret)

    def test_private_material_in_public_key_document_is_rejected(self):
        root = metadata.load(self.root).signed
        key = root.keys[root.roles["root"].keyids[0]]
        key.keyval["private"] = "not public metadata"
        with self.assertRaises(ValueError):
            metadata.root_safe(root)

    def test_online_roles_can_refresh_without_new_targets_approvals(self):
        targets, snapshot, timestamp = self.release()
        mirror = self.path / "mirror"
        repository.publish(self.roots, targets, snapshot, timestamp, self.artifact, mirror)
        for role, child, version in (("timestamp", snapshot, 2), ("snapshot", targets, 2), ("timestamp", None, 3)):
            draft, output = self.path / f"{role}-{version}.unsigned", self.path / f"{role}-{version}.json"
            metadata.role_draft(role, child or snapshot, self.root, draft, version)
            self.assemble(draft, self.roles[role], output)
            if role == "snapshot":
                snapshot = output
                continue
            timestamp = output
            repository.publish(self.roots, targets, snapshot, timestamp, self.artifact, mirror)
        _, _, docs, _, _ = repository.current(mirror)
        self.assertEqual([doc.signed.version for doc in docs], [1, 2, 3])
        with self.assertRaises(ValueError):
            repository.publish(self.roots, targets, snapshot, timestamp, self.artifact, mirror)

    def test_publisher_uses_the_verified_snapshot_not_reopened_input(self):
        paths = self.release()
        original = repository.release

        def mutate_after_verification(*args, **kwargs):
            result = original(*args, **kwargs)
            paths[0].write_bytes(b"untrusted later replacement")
            return result

        with patch.object(repository, "release", side_effect=mutate_after_verification):
            repository.publish(self.roots, *paths, self.artifact, self.path / "mirror")
        self.assertEqual(repository.current(self.path / "mirror")[3]["release_sequence"], 1)


if __name__ == "__main__":
    unittest.main()
