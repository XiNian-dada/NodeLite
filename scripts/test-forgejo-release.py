#!/usr/bin/env python3
"""Check release integrity and fail-closed draft handling without network access."""

import contextlib
import hashlib
import importlib.util
import io
import json
import tempfile
import unittest
import urllib.error
from pathlib import Path

spec = importlib.util.spec_from_file_location("publisher", Path(__file__).with_name("publish-forgejo-release.py"))
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)
COMMIT = "a" * 40
REPO = "owner/NodeLite"
SERVER = "https://forgejo.example"
TAG = "v1.2.3-rc.1"


class FakeClient:
    def __init__(self, *, existing=None, fail_upload=False):
        self.existing = existing
        self.fail_upload = fail_upload
        self.assets = []
        self.published = False
        self.created = False

    def request(self, method, path, data=None):
        if path.startswith("/commits/"):
            return {"sha": COMMIT}
        if method == "GET" and path.startswith("/releases/tags/"):
            if self.existing is not None:
                return self.existing
            raise urllib.error.HTTPError(path, 404, "missing", {}, None)
        if method == "POST":
            assert data["draft"] is True and data["prerelease"] is True
            self.created = True
            return {"draft": True, "target_commitish": COMMIT, "id": 1}
        if method == "GET":
            return self.assets
        if method == "PATCH":
            self.published = True
            return {"html_url": SERVER + "/release"}
        raise AssertionError((method, path))

    def upload(self, path, file):
        if self.fail_upload:
            raise OSError("interrupted upload")
        self.assets.append({"name": file.name, "size": file.stat().st_size})


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="nodelite-forgejo-release-")
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        names = [f"nodelite-{binary}-{target}" for binary in ["server", "agent", "updater"]
                 for target in ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"]]
        names += ["install-server.sh", "install-server-config.sh", "install-server-upgrade.sh",
                  "install-agent.sh", "install-agent-signed.sh", "THIRD_PARTY_NOTICES.md"]
        names += [f"nodelite-{binary}.cdx.json" for binary in ["proto", "server", "agent", "updater"]]
        for name in names:
            (self.directory / name).write_bytes(b"fixture")
        (self.directory / "BUILD_INFO.json").write_text(json.dumps({"commit": COMMIT, "tag": TAG,
            "repository": REPO, "server": SERVER, "signed_attestation": False}))
        self.manifest()

    def manifest(self):
        files = sorted(p for p in self.directory.iterdir() if p.name != "SHA256SUMS.txt")
        (self.directory / "SHA256SUMS.txt").write_text("".join(
            f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in files))

    def publish(self, client):
        with contextlib.redirect_stdout(io.StringIO()):
            publisher.publish(client, self.directory, TAG, COMMIT, REPO, SERVER)

    def test_complete_release_publishes_after_all_assets(self):
        client = FakeClient()
        self.publish(client)
        self.assertTrue(client.published)
        self.assertEqual(len(client.assets), 18)

    def test_corruption_is_rejected_before_draft_creation(self):
        client = FakeClient()
        (self.directory / "install-agent.sh").write_bytes(b"modified")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            self.publish(client)
        self.assertFalse(client.created)

    def test_interruption_keeps_draft(self):
        client = FakeClient(fail_upload=True)
        with self.assertRaises(OSError):
            self.publish(client)
        self.assertTrue(client.created)
        self.assertFalse(client.published)

    def test_published_and_partial_releases_are_preserved(self):
        for draft in [False, True]:
            client = FakeClient(existing={"draft": draft, "id": 1, "target_commitish": COMMIT})
            if draft:
                client.assets = [{"name": "existing", "size": 1}]
            with self.assertRaises(ValueError):
                self.publish(client)
            self.assertFalse(client.published)

    def test_duplicate_manifest_and_unrelated_files_fail(self):
        manifest = self.directory / "SHA256SUMS.txt"
        manifest.write_text(manifest.read_text() + manifest.read_text().splitlines()[0] + "\n")
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.publish(FakeClient())
        self.manifest()
        (self.directory / "unrelated.txt").write_text("preserve")
        with self.assertRaisesRegex(ValueError, "exact Linux"):
            self.publish(FakeClient())

    def test_token_cannot_be_redirected_to_another_origin(self):
        self.assertIsNone(publisher.NoRedirect().redirect_request(None, None, 302, "", {}, "https://other.example"))
        for server in ["http://forgejo.example", "https://user:secret@forgejo.example", "https://forgejo.example/path"]:
            with self.assertRaises(ValueError):
                publisher.Client(server, REPO, "fixture")


if __name__ == "__main__":
    unittest.main()
