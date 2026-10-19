"""POSIX CLI experiment: bound a complete network operation, including slow trickles."""

from contextlib import contextmanager
import signal


class NetworkDeadline(TimeoutError):
    pass


@contextmanager
def network_deadline(seconds=5):
    if signal.getitimer(signal.ITIMER_REAL) != (0.0, 0.0):
        raise RuntimeError("nested network deadline is not supported")

    def expired(*_):
        raise NetworkDeadline("lab network operation exceeded total deadline")

    previous = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, seconds)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)
