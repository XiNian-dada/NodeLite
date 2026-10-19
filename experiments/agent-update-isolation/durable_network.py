#!/usr/bin/env python3
"""Exercise durable trust floors against real signed metadata and HTTP failures."""

from contextlib import contextmanager
import hashlib
from http.server import ThreadingHTTPServer
import json
from pathlib import Path
import tempfile
import threading
import time

from cache_store import CacheInvalid
from durable_client import DurableClient
from releases import RELEASES, TARGET
from tuf_network import Mirror, rejected
from tuf_repo import Repository
from vm import WORK


@contextmanager
def mirror(repo):
    server = ThreadingHTTPServer(("127.0.0.1", 0), Mirror)
    server.root, server.requests, server.overrides, server.truncate = repo.directory, [], {}, set()
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server, f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def scenario(name, directory, payloads):
    repo = Repository(directory / "repo")
    repo.publish(1, payloads[0])
    with mirror(repo) as (server, base):
        client = DurableClient(directory / "cache", base, repo.bootstrap)
        client.store.provision(time.time())
        assert client.download().read_bytes() == payloads[0]
        before = client.store.load()
        result = {"name": name, "passed": True}
        if name == "target_failure_preserves_new_floor":
            target = repo.publish(2, payloads[1])
            server.overrides[target] = b"bad"
            result["download_rejected_by"] = rejected(client)
            current = client.store.load()
            assert current["sequence"] == 2 and current["artifact"] == before["artifact"]
            repo.publish(1, payloads[0], version=3)
            result["downgrade_rejected_by"] = rejected(client)
            repo.publish(2, payloads[1], version=4)
            server.overrides.clear()
            assert client.download().read_bytes() == payloads[1]
        elif name == "root_revocation_survives_later_failure":
            revoked = repo.keys["targets"]
            repo.rotate()
            repo.publish(2, payloads[1], target_keys=revoked)
            result["first_rejected_by"] = rejected(client)
            active = client.store.generation(client.store.load()["active"]) / "metadata"
            root = json.loads((active / "root.json").read_text())
            assert root["signed"]["version"] == 2
            (repo.directory / "metadata/2.root.json").unlink()
            repo.publish(2, payloads[1], version=3, target_keys=revoked)
            result["replay_rejected_by"] = rejected(client)
            repo.publish(2, payloads[1], version=4)
            assert client.download().read_bytes() == payloads[1]
        elif name in {"missing_journal", "corrupt_journal", "corrupt_metadata", "corrupt_object"}:
            if name == "missing_journal":
                client.store.state_path.unlink()
            elif name == "corrupt_journal":
                client.store.state_path.write_bytes(b"{")
            elif name == "corrupt_metadata":
                (client.store.generation(before["active"]) / "metadata/targets.json").write_bytes(b"{}")
            else:
                client.store.artifact(before).write_bytes(b"corrupt")
            requests = len(server.requests)
            result["rejected_by"] = rejected(client, (CacheInvalid,))
            assert len(server.requests) == requests, "corruption was silently refetched"
        else:
            raise ValueError("unknown durable scenario")
        result["http_requests"] = len(server.requests)
        return result


def main():
    if Path.cwd().resolve() != WORK.parents[1].resolve():
        raise RuntimeError("run from compiler ~/NodeLite only")
    output = WORK / "durable-network-evidence.json"
    output.unlink(missing_ok=True)
    payloads = [(RELEASES / version / f"nodelite-agent-{TARGET}").read_bytes()
                for version in ("3.0.3", "3.0.4")]
    results = []
    with tempfile.TemporaryDirectory(prefix="durable-network-", dir=WORK) as temporary:
        for name in ("target_failure_preserves_new_floor", "root_revocation_survives_later_failure",
                     "missing_journal", "corrupt_journal", "corrupt_metadata", "corrupt_object"):
            result = scenario(name, Path(temporary) / name, payloads)
            results.append(result)
            print(json.dumps(result), flush=True)
    output.write_text(json.dumps({"complete": True, "scenarios": results,
        "source_sha256": {name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest()
                          for name in ("durable_network.py", "durable_client.py", "cache_store.py")}}, indent=2) + "\n")


if __name__ == "__main__":
    main()
