#!/usr/bin/env python3
"""Bounded HTTP/proxy fault experiments using actual sockets and signed official payloads."""

import hashlib
from http.server import ThreadingHTTPServer
import json
from pathlib import Path
import tempfile
import threading
import time

from tuf.api.exceptions import DownloadError, RepositoryError
from urllib3.exceptions import HTTPError

from network_deadline import NetworkDeadline
from releases import RELEASES, TARGET
from tuf_client import Client
from tuf_network import Mirror
from tuf_repo import Repository
from vm import WORK


class FaultMirror(Mirror):
    def do_GET(self):
        if self.path != "/metadata/timestamp.json":
            return super().do_GET()
        self.server.requests.append(self.path)
        fault = self.server.fault
        try:
            if fault == "reset":
                self.connection.close()
                return
            if fault in {"http_503", "proxy_407", "not_found"}:
                self.send_error({"http_503": 503, "proxy_407": 407, "not_found": 404}[fault])
                return
            if fault == "redirect_loop":
                self.send_response(302)
                self.send_header("Location", "/metadata/timestamp.json")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            if fault == "no_headers":
                time.sleep(7)
                return
            self.send_response(200)
            self.send_header("Content-Length", "10000")
            self.end_headers()
            for _ in range(100):
                self.wfile.write(b" ")
                self.wfile.flush()
                time.sleep(0.1)
        except OSError:
            pass


def main():
    if Path.cwd().resolve() != WORK.parents[1].resolve():
        raise RuntimeError("run from compiler ~/NodeLite only")
    output = WORK / "network-resilience-evidence.json"
    output.unlink(missing_ok=True)
    payload = (RELEASES / "3.0.4" / f"nodelite-agent-{TARGET}").read_bytes()
    results = []
    with tempfile.TemporaryDirectory(prefix="network-resilience-", dir=WORK) as temporary:
        root = Path(temporary)
        repo = Repository(root / "repo")
        repo.publish(1, payload)
        for fault in ("reset", "http_503", "proxy_407", "not_found", "redirect_loop", "no_headers", "slow_body"):
            server = ThreadingHTTPServer(("127.0.0.1", 0), FaultMirror)
            server.root, server.requests, server.fault = repo.directory, [], fault
            server.overrides, server.truncate = {}, set()
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            client = Client(root / fault, f"http://127.0.0.1:{server.server_port}", repo.bootstrap)
            started = time.monotonic()
            try:
                try:
                    client.download()
                except (DownloadError, RepositoryError, HTTPError, NetworkDeadline, ConnectionError) as error:
                    rejected = type(error).__name__
                else:
                    raise AssertionError("faulty mirror unexpectedly succeeded")
                duration = time.monotonic() - started
                assert duration < 6.5, "network deadline was not bounded"
                assert not (client.directory / "release-high.json").exists()
                result = {"fault": fault, "passed": True, "elapsed_secs": round(duration, 3),
                          "rejected_by": rejected, "http_requests": len(server.requests)}
                results.append(result)
                print(json.dumps(result), flush=True)
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=2)
    output.write_text(json.dumps({"complete": True, "scenarios": results,
        "source_sha256": {name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest()
                          for name in ("network_deadline.py", "resilience_network.py", "tuf_client.py")}}, indent=2) + "\n")


if __name__ == "__main__":
    main()
