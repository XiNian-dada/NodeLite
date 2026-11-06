#!/usr/bin/env python3
"""Exercise real installer argument parsing before any privileged or network action."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent


class InstallerVersions(unittest.TestCase):
    def parse(self, role, *args, **variables):
        source = (SCRIPTS / f"install-{role}.sh").read_text()
        marker = 'mark_step "checking privileges"' if role == "server" else '[ "$(id -u)" -eq 0 ]'
        source = source[:source.index(marker)]
        source += '\nprintf "%s\\n" "$VERSION" "$BASE_URL" "$MODE"\n'
        env = {k: v for k, v in os.environ.items() if not k.startswith("NODELITE_")}
        env.update({f"NODELITE_{role.upper()}_{k}": v for k, v in variables.items()})
        with tempfile.TemporaryDirectory(prefix="nodelite-version-test-") as directory:
            script = Path(directory) / "parse.sh"
            script.write_text(source)
            return subprocess.run(["sh", str(script), *args], env=env, text=True, capture_output=True)

    def test_default_stays_on_stable(self):
        for role in ("agent", "server"):
            result = self.parse(role)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.splitlines(), ["", "https://github.com/XiNian-dada/NodeLite/releases/latest/download", "auto"])

    def test_rc_flag_overrides_environment_without_falling_back_to_latest(self):
        for role in ("agent", "server"):
            result = self.parse(role, "--version", "v3.1.0-rc.1", "--mode", "upgrade", VERSION="v3.0.4")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.splitlines(), ["v3.1.0-rc.1", "https://github.com/XiNian-dada/NodeLite/releases/download/v3.1.0-rc.1", "upgrade"])

    def test_environment_version_remains_supported(self):
        for role in ("agent", "server"):
            result = self.parse(role, VERSION="v3.1.0-rc.1", MODE="upgrade")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("/download/v3.1.0-rc.1\nupgrade\n", result.stdout)

    def test_mirror_is_preserved_regardless_of_argument_order(self):
        for role in ("agent", "server"):
            for args in (("--base-url", "https://mirror.example/v3", "--version", "v3.1.0-rc.1"),
                         ("--version", "v3.1.0-rc.1", "--base-url", "https://mirror.example/v3")):
                result = self.parse(role, *args, BASE_URL="https://old.example/v3")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.splitlines()[1], "https://mirror.example/v3")
            result = self.parse(role, "--version", "v3.1.0-rc.1", BASE_URL="https://old.example/v3")
            self.assertEqual(result.stdout.splitlines()[1], "https://old.example/v3")

    def test_bad_or_missing_arguments_fail_before_privileged_actions(self):
        for role in ("agent", "server"):
            for args in (("--version",), ("--version", ""), ("--version", "../v3.1"),
                         ("--version", "v3.1?x"), ("--version", "v3.1\n"), ("--unknown",)):
                with self.subTest(role=role, args=args):
                    self.assertNotEqual(self.parse(role, *args).returncode, 0)

    def test_help_works_without_root(self):
        for role in ("agent", "server"):
            result = subprocess.run(["sh", str(SCRIPTS / f"install-{role}.sh"), "--help"], text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("--version", result.stdout)


if __name__ == "__main__":
    unittest.main()
