"""POSIX deadline restoration checks without sockets or wall-clock changes."""

import signal
import unittest

from network_deadline import NetworkDeadline, network_deadline


class DeadlineTests(unittest.TestCase):
    def test_signal_raises_a_bounded_operation_error_and_restores_handler(self):
        previous = signal.getsignal(signal.SIGALRM)
        with self.assertRaises(NetworkDeadline), network_deadline():
            signal.raise_signal(signal.SIGALRM)
        self.assertEqual(signal.getsignal(signal.SIGALRM), previous)
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))

    def test_unrelated_exception_cancels_the_pending_alarm(self):
        with self.assertRaises(ValueError), network_deadline():
            raise ValueError("ordinary failure")
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))

    def test_nested_operation_cannot_clobber_the_outer_timer(self):
        with network_deadline():
            with self.assertRaises(RuntimeError), network_deadline():
                self.fail("nested deadline was allowed")
            self.assertGreater(signal.getitimer(signal.ITIMER_REAL)[0], 0)


if __name__ == "__main__":
    unittest.main()
