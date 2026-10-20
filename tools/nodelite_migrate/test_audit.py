"""Stale, mismatched or malformed local health must never satisfy deployment admission."""

import io
import json
import unittest
from unittest.mock import patch

from tools.nodelite_migrate import transaction
from tools.nodelite_migrate.audit import health_matches


class Health(unittest.TestCase):
    def test_fresh_matching_process_only(self):
        process = {"MainPID": "123", "InvocationID": "a" * 32}
        value = {"schema": 1, "pid": 123, "invocation": "a" * 32, "executable_sha256": "b" * 64,
                 "sample_counter": 4, "collected_at_ms": 1000, "sample_boottime_ms": 2000}
        self.assertTrue(health_matches(value, process, "b" * 64, 1001, 2001))
        for changes in ({"pid": 124}, {"invocation": "c" * 32}, {"sample_counter": 0}, {"schema": 2},
                        {"collected_at_ms": 2000}, {"sample_boottime_ms": -999999}, {"pid": []}):
            self.assertFalse(health_matches({**value, **changes}, process, "b" * 64, 1001, 2001))
        self.assertFalse(health_matches(value, process, "c" * 64, 1001, 2001))
        self.assertFalse(health_matches(value, process, "b" * 64, 999999, 999999))
        self.assertFalse(health_matches({}, process, "b" * 64, 1001, 2001))

    def migration_health(self, interval):
        clock = [0.0]
        binary = b"test process image"
        expected = transaction.digest(binary)
        process = {"MainPID": "123", "InvocationID": "a" * 32, "ActiveState": "active"}

        def evidence(*_):
            sample = int(clock[0] // interval) * interval if interval else 0
            return json.dumps({"schema": 1, "pid": 123, "invocation": "a" * 32,
                "executable_sha256": expected, "sample_counter": 1 + sample,
                "collected_at_ms": 1000000 + sample * 1000,
                "sample_boottime_ms": 1000000 + sample * 1000}).encode()

        def advance(seconds):
            clock[0] = round(clock[0] + seconds, 3)

        with patch.object(transaction, "properties", return_value=process), \
                patch.object(transaction, "read", side_effect=evidence), \
                patch("builtins.open", side_effect=lambda *_: io.BytesIO(binary)), \
                patch.object(transaction.time, "CLOCK_BOOTTIME", 7, create=True), \
                patch.object(transaction.time, "monotonic", side_effect=lambda: clock[0]), \
                patch.object(transaction.time, "time", side_effect=lambda: 1000 + clock[0]), \
                patch.object(transaction.time, "clock_gettime", side_effect=lambda _: 1000 + clock[0]), \
                patch.object(transaction.time, "sleep", side_effect=advance):
            transaction.healthy(expected, 60)

    def test_supported_intervals_establish_advancing_health(self):
        for interval in (1, 5, 10):
            with self.subTest(interval=interval):
                self.migration_health(interval)

    def test_stalled_or_stale_samples_cannot_pass_migration(self):
        for interval in (None, 20, 30):
            with self.subTest(interval=interval), self.assertRaises(RuntimeError):
                self.migration_health(interval)
