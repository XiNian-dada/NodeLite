"""No VM or credentials needed: lab observer timestamp compatibility regressions."""

import unittest
from unittest.mock import patch

import cold_support
from cold_support import collection_time


class CollectionTimeTests(unittest.TestCase):
    def test_chrono_nanoseconds_are_accepted_without_inventing_freshness(self):
        sample = collection_time("2026-10-19T03:20:44.215558495Z")
        self.assertEqual(sample, collection_time("2026-10-19T03:20:44.215558Z"))

    def test_optional_or_short_fraction_is_supported(self):
        base = collection_time("2026-10-19T03:20:44Z")
        self.assertAlmostEqual(collection_time("2026-10-19T03:20:44.1Z") - base, 0.1, places=5)

    def test_non_utc_or_malformed_timestamp_is_rejected(self):
        for value in ("", "now", "2026-10-19T03:20:44+08:00", "2026-10-19T03:20:44.1234567890Z"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                collection_time(value)

    def test_observer_publishes_readable_inode_without_post_rename_ownership_changes(self):
        process = {"InvocationID": "invocation", "digest": "old", "pid": 42, "ActiveState": "active"}
        status = {"snapshot": {"collected_at": "timestamp"}, "online": True,
                  "identity": {"agent_version": cold_support.OLD}}
        with patch.object(cold_support, "guard"), \
                patch.object(cold_support, "probe", return_value=process), \
                patch.object(cold_support, "api", return_value=status), \
                patch.object(cold_support, "collection_time", return_value=10.5), \
                patch.object(cold_support.Path, "read_text", return_value='{"old":"old","new":"new"}'), \
                patch.object(cold_support.Path, "exists", return_value=False), \
                patch.object(cold_support, "atomic_bytes") as publish, \
                patch.object(cold_support, "own") as own, \
                patch.object(cold_support.time, "time", side_effect=[10, 10, 11]), \
                patch.object(cold_support.time, "sleep", side_effect=StopIteration):
            with self.assertRaises(StopIteration):
                cold_support.observe()
            self.assertEqual(publish.call_count, 1)
            self.assertEqual(publish.call_args.args[2], 0o640)
            own.assert_not_called()


if __name__ == "__main__":
    unittest.main()
