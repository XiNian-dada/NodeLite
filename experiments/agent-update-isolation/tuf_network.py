#!/usr/bin/env python3
"""Real HTTP adversarial tests on compiler loopback; keys exist only in process memory."""

import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.metadata
import json
from pathlib import Path
import tempfile
import threading

from tuf.api.exceptions import DownloadError, RepositoryError
from urllib3.exceptions import HTTPError

from releases import RELEASES, TARGET as ARCH
from tuf_client import Client, LabPolicyError
from tuf_repo import Repository
from vm import WORK


class Mirror(BaseHTTPRequestHandler):
    def do_GET(self):
        self.server.requests.append(self.path)
        path = (self.server.root / self.path.lstrip("/")).resolve()
        if not path.is_relative_to(self.server.root) or not path.is_file():
            self.send_error(404)
            return
        original = path.read_bytes()
        data = self.server.overrides.get(self.path, original)
        self.send_response(200)
        self.send_header("Content-Length", str(len(original) if self.path in self.server.truncate else len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_):
        pass


def rejected(client, expected=(RepositoryError, DownloadError, HTTPError, LabPolicyError)):
    try:
        client.download()
    except expected as error:
        return type(error).__name__
    raise AssertionError("malicious mirror content was accepted")


def scenario(name, directory, payloads):
    repo = Repository(directory / "repo")
    server = ThreadingHTTPServer(("127.0.0.1", 0), Mirror)
    server.root, server.requests, server.overrides, server.truncate = repo.directory, [], {}, set()
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    client = Client(directory / "client", f"http://127.0.0.1:{server.server_port}", repo.bootstrap)
    result = {"name": name, "passed": True}
    try:
        target = repo.publish(1, payloads[0])
        if name == "success":
            repo.publish(2, payloads[1])
            assert hashlib.sha256(Path(client.download()).read_bytes()).digest() == hashlib.sha256(payloads[1]).digest()
        elif name in {"tampered_target", "truncated_target", "oversize_target"}:
            server.overrides[target] = {"tampered_target": b"X" + payloads[0][1:],
                                        "truncated_target": payloads[0][:512],
                                        "oversize_target": payloads[0] + b"X"}[name]
            if name == "truncated_target":
                server.truncate.add(target)
            result["rejected_by"] = rejected(client)
        elif name in {"one_targets_signer", "expired", "wrong_scope"}:
            kwargs = {"one_targets_signer": {"target_keys": repo.keys["targets"][:1]},
                      "expired": {"expires": -60},
                      "wrong_scope": {"custom": {"product": "not-nodelite"}}}[name]
            repo.publish(2, payloads[1], **kwargs)
            result["rejected_by"] = rejected(client)
        elif name in {"timestamp_replay", "signed_release_downgrade", "same_release_equivocation", "mix_and_match"}:
            old_timestamp = (repo.directory / "metadata/timestamp.json").read_bytes()
            old_targets = (repo.directory / "metadata/1.targets.json").read_bytes()
            client.download()
            repo.publish(2, payloads[1])
            client.download()
            if name == "timestamp_replay":
                server.overrides["/metadata/timestamp.json"] = old_timestamp
            elif name == "signed_release_downgrade":
                repo.publish(1, payloads[0], version=3)
            elif name == "same_release_equivocation":
                repo.publish(2, payloads[0], version=3)
            else:
                repo.publish(3, payloads[1])
                server.overrides["/metadata/3.targets.json"] = old_targets
            result["rejected_by"] = rejected(client)
        elif name in {"root_rotation", "root_old_threshold", "root_new_threshold", "revoked_targets"}:
            client.download()
            revoked = repo.keys["targets"]
            repo.rotate(old_count=1 if name == "root_old_threshold" else 2,
                        new_count=1 if name == "root_new_threshold" else 2)
            repo.publish(2, payloads[1], target_keys=revoked if name == "revoked_targets" else None)
            if name == "root_rotation":
                assert Path(client.download()).read_bytes() == payloads[1]
            else:
                result["rejected_by"] = rejected(client)
        elif name == "oversize_metadata":
            server.overrides["/metadata/timestamp.json"] = b" " * 17000
            result["rejected_by"] = rejected(client)
        else:
            raise ValueError("unknown TUF network scenario")
        result["http_requests"] = len(server.requests)
        assert result["http_requests"] > 0, "test bypassed real HTTP"
        return result
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=3)


def main():
    if Path.cwd().resolve() != WORK.parents[1].resolve():
        raise RuntimeError("run from compiler ~/NodeLite only")
    (WORK / "tuf-network-evidence.json").unlink(missing_ok=True)
    payloads = [(RELEASES / version / f"nodelite-agent-{ARCH}").read_bytes()
                for version in ("3.0.3", "3.0.4")]
    results = []
    with tempfile.TemporaryDirectory(prefix="tuf-lab-", dir=WORK) as temporary:
        for name in ("success", "tampered_target", "truncated_target", "oversize_target",
                     "one_targets_signer", "expired", "wrong_scope", "timestamp_replay",
                     "signed_release_downgrade", "same_release_equivocation", "mix_and_match", "root_rotation",
                     "root_old_threshold", "root_new_threshold", "revoked_targets", "oversize_metadata"):
            result = scenario(name, Path(temporary) / name, payloads)
            results.append(result)
            print(json.dumps(result), flush=True)
    evidence = {"dependencies": {name: importlib.metadata.version(name) for name in (
        "tuf", "securesystemslib", "cryptography", "urllib3")}, "scenarios": results,
        "source_sha256": {name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest()
                          for name in ("tuf_network.py", "tuf_client.py", "tuf_repo.py", "lab-requirements.txt")},
        "official_agent_sha256": [hashlib.sha256(payload).hexdigest() for payload in payloads]}
    (WORK / "tuf-network-evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
