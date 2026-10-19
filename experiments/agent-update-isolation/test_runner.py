"""Offline export regression: no compilation, SSH or systemd operation is performed."""

import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run_real


class EvidenceExportTests(unittest.TestCase):
    def test_worker_loop_cannot_change_the_evidence_destination(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            binaries = work / "binaries"
            binaries.mkdir()
            names = ("agent-old", "agent-new", "real-test-runner")
            for name in names:
                (binaries / name).write_bytes(name.encode())
            (work / "real-agent-results.json").write_text(json.dumps([{"passed": True}] * 8))
            worker = work / "lab_io.py"
            worker.write_bytes(b"retained")
            with patch.object(run_real, "WORK", work), \
                    patch.object(run_real, "PROJECT", Path.cwd()), \
                    patch.object(run_real.pwd, "getpwuid", return_value=SimpleNamespace(pw_name="compiler")), \
                    patch.object(run_real, "build", return_value=binaries), \
                    patch.object(run_real, "guest"), \
                    patch.object(run_real, "command", return_value=SimpleNamespace(stdout="")), \
                    patch("sys.argv", ["run_real.py", "--official"]):
                run_real.main()
            evidence = json.loads((work / "official-real-evidence.json").read_text())
            self.assertEqual(len(evidence["scenarios"]), 8)
            self.assertEqual(set(evidence["binaries"]), set(names))
            self.assertEqual(worker.read_bytes(), b"retained")


if __name__ == "__main__":
    unittest.main()
