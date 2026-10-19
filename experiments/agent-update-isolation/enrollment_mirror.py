"""Loopback-only public mirror; faults are test instrumentation, never installed by migration."""

from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from functools import partial
import time

ROOT = Path("/var/lib/nlup-enrollment-public")


class Mirror(SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        fault = (ROOT / "fault").read_text() if (ROOT / "fault").exists() else ""
        if fault == "tampered" and self.path.startswith("/targets/"):
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"invalid artifact")
            return
        if fault == "slow" and self.path.endswith("timestamp.json"):
            time.sleep(12)
        return super().do_GET()


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", 18094), partial(Mirror, directory=str(ROOT / "selected"))).serve_forever()
