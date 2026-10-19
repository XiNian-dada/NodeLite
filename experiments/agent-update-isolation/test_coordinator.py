"""Offline state-machine regressions; no system units, users or protected paths are touched."""

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
        state = {"boot_id": "boot", "deadline": 100, "phase": "stopping"}
        with patch.object(updater, "boot_id", return_value="boot"), \
                patch.object(updater.time, "monotonic", return_value=10), \
                patch.object(updater, "properties", return_value={"ActiveState": "inactive",
                    "MainPID": "0", "ControlGroup": "/system.slice/nlup-lab-agent.service"}), \
                patch.object(updater, "Path") as path, patch.object(updater, "marker") as marker:
            processes = path.return_value.__truediv__.return_value.__truediv__.return_value
            processes.exists.return_value = True
            processes.read_text.return_value = "123\n"
            updater.stopping(state)
            self.assertEqual(state["phase"], "stopping")
            marker.assert_called_once_with("stop")


if __name__ == "__main__":
    unittest.main()
