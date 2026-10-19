"""Offline state-machine regressions; no system units, users or protected paths are touched."""

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import updater


class HealthProgressTests(unittest.TestCase):
    def setUp(self):
        self.state = {"phase": "trial", "generation": 1, "boot_id": "boot",
                      "from": "old", "to": "new", "to_sequence": 2, "deadline": 100,
                      "old_invocation": "old-run", "stable_since": None, "stable_sample": None}
        self.health = {"pid": 42, "digest": "new", "sequence": 2, "time": 1000}
        self.process = {"ActiveState": "active", "MainPID": "42", "InvocationID": "new-run"}
        self.wall, self.monotonic = 1000, 10
        for target, value in (("boot_id", lambda: "boot"), ("active_digest", lambda: "new"),
                              ("heartbeat", lambda: self.health), ("properties", lambda _: self.process),
                              ("atomic_json", lambda *_: None)):
            context = patch.object(updater, target, value)
            context.start()
            self.addCleanup(context.stop)
        for name, value in (("time", lambda: self.wall), ("monotonic", lambda: self.monotonic)):
            context = patch.object(updater.time, name, value)
            context.start()
            self.addCleanup(context.stop)

    def test_frozen_evidence_cannot_commit_even_while_recent(self):
        updater.reconcile(self.state)
        self.monotonic, self.wall = 13, 1001
        updater.reconcile(self.state)
        self.assertEqual(self.state["phase"], "trial")

    def test_advancing_samples_and_stable_process_commit(self):
        updater.reconcile(self.state)
        self.monotonic, self.wall, self.health["time"] = 13, 1003, 1003
        updater.reconcile(self.state)
        self.assertEqual(self.state["phase"], "committed")

    def test_new_invocation_restarts_the_stability_window(self):
        updater.reconcile(self.state)
        self.monotonic, self.wall, self.health["time"] = 13, 1003, 1003
        self.process["InvocationID"] = "another-run"
        updater.reconcile(self.state)
        self.assertEqual(self.state["phase"], "trial")
        self.assertEqual(self.state["stable_since"], 13)
        self.assertEqual(self.state["stable_sample"], 1003)

    def test_future_or_stale_samples_reset_stability(self):
        for sample in (999, 1004):
            with self.subTest(sample=sample):
                self.wall, self.monotonic, self.health["time"] = 1000, 10, 1000
                updater.reconcile(self.state)
                self.wall, self.monotonic, self.health["time"] = 1003, 13, sample
                updater.reconcile(self.state)
                self.assertEqual(self.state["phase"], "trial")
                self.assertIsNone(self.state["stable_since"])


class StopBarrierTests(unittest.TestCase):
    def test_inactive_main_process_is_not_enough_if_cgroup_has_children(self):
        state = {"boot_id": "boot", "deadline": 100, "phase": "stopping", "stop_exit": "1"}
        process = {"ActiveState": "inactive", "MainPID": "0",
                   "ControlGroup": "/system.slice/nlup-lab-agent.service"}
        stopped = {"ActiveState": "inactive", "Result": "success", "ExecMainExitTimestampMonotonic": "2"}
        with patch.object(updater, "boot_id", return_value="boot"), \
                patch.object(updater.time, "monotonic", return_value=10), \
                patch.object(updater, "properties", side_effect=lambda unit: stopped if "bridge" in unit else process), \
                patch.object(updater, "Path") as path, patch.object(updater, "marker") as marker:
            processes = path.return_value.__truediv__.return_value.__truediv__.return_value
            processes.exists.return_value = True
            processes.read_text.return_value = "123\n"
            updater.stopping(state)
            self.assertEqual(state["phase"], "stopping")
            marker.assert_called_once_with("stop")

    def test_no_switch_until_the_fixed_root_stop_action_has_completed(self):
        state = {"boot_id": "boot", "deadline": 100, "phase": "stopping", "stop_exit": "1"}
        with patch.object(updater, "boot_id", return_value="boot"), \
                patch.object(updater.time, "monotonic", return_value=10), \
                patch.object(updater, "properties", return_value={"ActiveState": "inactive",
                    "MainPID": "0", "ControlGroup": "", "Result": "success",
                    "ExecMainExitTimestampMonotonic": "1"}), \
                patch.object(updater, "marker") as marker, patch.object(updater, "pointer") as pointer:
            updater.stopping(state)
            self.assertEqual(state["phase"], "stopping")
            marker.assert_called_once_with("stop")
            pointer.assert_not_called()

    def test_unconfirmed_stop_timeout_requires_manual_recovery_without_switching(self):
        state = {"boot_id": "boot", "deadline": 10, "phase": "stopping", "stop_exit": "1"}
        with patch.object(updater, "boot_id", return_value="boot"), \
                patch.object(updater.time, "monotonic", return_value=11), \
                patch.object(updater, "properties", return_value={"ActiveState": "activating",
                    "MainPID": "0", "ControlGroup": "", "ExecMainExitTimestampMonotonic": "1"}), \
                patch.object(updater, "BRIDGE") as bridge, \
                patch.object(updater, "manual") as manual, patch.object(updater, "pointer") as pointer:
            updater.stopping(state)
            manual.assert_called_once_with("old service stop was not confirmed")
            bridge.__truediv__.return_value.unlink.assert_called_once_with(missing_ok=True)
            pointer.assert_not_called()


class BootBridgeTests(unittest.TestCase):
    def test_cold_boot_discards_only_fixed_action_markers_without_following_links(self):
        parent = Path(__file__).resolve().parent / ".io-tests"
        parent.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=parent) as directory:
            bridge = Path(directory)
            (bridge / "unrelated").write_bytes(b"retained")
            (bridge / "stop").symlink_to(bridge / "unrelated")
            for name in ("restart", "preflight"):
                (bridge / name).write_bytes(b"stale request")
            with patch.object(updater, "BRIDGE", bridge):
                updater.clear_bridge_requests()
            self.assertEqual(list(bridge.iterdir()), [bridge / "unrelated"])
            self.assertEqual((bridge / "unrelated").read_bytes(), b"retained")


if __name__ == "__main__":
    unittest.main()
