#!/usr/bin/env python3
"""Run the real enrollment shell branches with disposable downloads, without root changes."""

import hashlib
import io
import os
from pathlib import Path
import shlex
import subprocess
import tarfile
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent
INSTALLER = (SCRIPTS / "install-agent.sh").read_text()
HELPER = (SCRIPTS / "install-agent-signed.sh").read_text()
FAIL = 'fail() { printf "%s\\n" "$*" >&2; exit 1; }\n'


def sha(data):
    return hashlib.sha256(data).hexdigest()


class EnrollmentPins(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name)

    def run_shell(self, code, **values):
        variables = "".join(f"{name}={shlex.quote(str(value))}\n" for name, value in values.items())
        return subprocess.run(["sh", "-c", "set -eu\n" + FAIL + variables + code],
                              cwd=self.path, text=True, capture_output=True)

    def test_explicit_version_and_both_pins_required_before_latest_resolution(self):
        start = INSTALLER.index('if [ "$SIGNED_UPDATES" -eq 1 ]; then', INSTALLER.index('ARTIFACT_NAME='))
        end = INSTALLER.index('if [ -n "$BINARY_URL" ]; then', start)
        branch = INSTALLER[start:end]
        self.assertNotIn("resolve_release_base_url", branch)
        good = dict(SIGNED_UPDATES=1, VERSION="v3.0.5", ENROLLMENT_SHA256="a" * 64, SIGNED_HELPER_SHA256="b" * 64)
        self.assertEqual(self.run_shell(branch, **good).returncode, 0)
        for key, bad in (("VERSION", ""), ("ENROLLMENT_SHA256", ""), ("SIGNED_HELPER_SHA256", ""),
                         ("ENROLLMENT_SHA256", "A" * 64), ("SIGNED_HELPER_SHA256", "0" * 63)):
            with self.subTest(key=key, bad=bad):
                self.assertNotEqual(self.run_shell(branch, **{**good, key: bad}).returncode, 0)

    def test_helper_pin_checked_before_sourcing_even_with_matching_remote_checksum(self):
        start = INSTALLER.index('if [ "$SIGNED_UPDATES" -eq 1 ]; then\n  for source_url')
        end = INSTALLER.index('\nprepare_service_account', start)
        branch = INSTALLER[start:end]
        helper = b'printf executed > helper-executed\nprepare_signed_update() { :; }\nfinish_signed_update() { :; }\n'
        (self.path / "helper").write_bytes(helper)
        stub = '''
mktemp() { printf '%s' "$PWD/download"; }
fetch_expected_sha256() { EXPECTED_SHA256="$REMOTE_SHA"; }
fetch_bounded() { cp helper "$4"; }
calculate_sha256() { printf '%s' "$REMOTE_SHA"; }
verify_release_attestation() { :; }
'''
        values = dict(SIGNED_UPDATES=1, BASE_URL="https://release.example.invalid/v3.0.5", CHECKSUMS_URL="",
                      VERSION="v3.0.5", REMOTE_SHA=sha(helper), SIGNED_HELPER_SHA256="f" * 64, existing_install=1)
        rejected = self.run_shell(stub + branch, **values)
        self.assertNotEqual(rejected.returncode, 0, rejected.stderr)
        self.assertIn("independently trusted pin", rejected.stderr)
        self.assertFalse((self.path / "helper-executed").exists())
        accepted = self.run_shell(stub + branch, **{**values, "SIGNED_HELPER_SHA256": sha(helper)})
        self.assertEqual(accepted.returncode, 0, accepted.stderr)
        self.assertTrue((self.path / "helper-executed").exists())

    def test_archive_pin_checked_before_extracting_or_executing_archive_code(self):
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w:gz") as archive:
            code = b'raise RuntimeError("untrusted code executed")'
            item = tarfile.TarInfo("tools/nodelite_migrate/installer.py")
            item.size = len(code)
            archive.addfile(item, io.BytesIO(code))
        archive_bytes = stream.getvalue()
        (self.path / "source.tar.gz").write_bytes(archive_bytes)
        stage = self.path / "stage"
        stage.mkdir()
        # Skip host provisioning only. Exercise the real download, hash, extraction and import sequence.
        start = HELPER.index('  enrollment_asset=')
        end = HELPER.index('\n}\n', start)
        stub = '''
fetch_expected_sha256() { EXPECTED_SHA256="$REMOTE_SHA"; }
curl() { cp source.tar.gz "$SIGNED_STAGE/package.tar.gz"; }
calculate_sha256() { printf '%s' "$REMOTE_SHA"; }
'''
        result = self.run_shell(stub + HELPER[start:end], TARGET="x86_64-unknown-linux-musl",
                                SIGNED_STAGE=stage, BASE_URL="https://release.example.invalid/v3.0.5",
                                REMOTE_SHA=sha(archive_bytes), ENROLLMENT_SHA256="f" * 64)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("independently trusted pin", result.stderr)
        self.assertEqual([p.name for p in stage.iterdir()], ["package.tar.gz"])


if __name__ == "__main__":
    unittest.main()
