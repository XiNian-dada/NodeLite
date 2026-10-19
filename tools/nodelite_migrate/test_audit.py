"""Stale, mismatched or malformed local health must never satisfy deployment admission."""

import unittest

from .audit import health_matches


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
