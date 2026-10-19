#!/usr/bin/env python3
"""Guest loopback HTTP faults with no credentials, logging or arbitrary upstream fetches."""

from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import time

ROOT = Path("/var/lib/nlup-rust-repository")


class Mirror(SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        fault = (ROOT / "fault").read_text() if (ROOT / "fault").exists() else ""
        if fault == "root_503" and self.path.endswith(".root.json"):
            self.send_error(503)
            return
        if fault == "slow_timestamp" and self.path.endswith("timestamp.json"):
            self.send_response(200)
            self.send_header("Content-Length", "10000")
            self.end_headers()
            try:
                for _ in range(150):
                    self.wfile.write(b" ")
                    self.wfile.flush()
                    time.sleep(0.1)
            except OSError:
                pass
            return
        return super().do_GET()


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", 18092), partial(Mirror, directory=str(ROOT / "current"))).serve_forever()
