#!/usr/bin/env python3
"""Actual ENOSPC and inode exhaustion for downloads and released Agent token writes."""

import argparse
import json
import os
import time

from cache_store import Store
from cold_support import CONFIG, OLD, LabApiError, api, online, setup, stop_all
from lab_io import atomic_bytes
from real_guest import own, probe
from resilience_guest import bootstrap, invoke, setup as setup_mirror
from resilience_volume import capacity, exhaust, release, volume
from setup_guest import LIB, directory, guard, run
from test_guest import systemctl


def cache_case(kind, partial=False):
    with volume() as mountpoint:
        cache = mountpoint / "cache"
        directory(cache, "nlup-updater", "nlup-updater", 0o700)
        Store(cache, bootstrap()).provision(time.time())
        for path in cache.rglob("*"):
            own(path, "nlup-updater", 0o700 if path.is_dir() else 0o600, "nlup-updater")
        first = invoke(cache, "one")
        assert first["success"]
        fault, paths = exhaust(mountpoint, kind, 512 * 1024 if partial else 0)
        rejected = invoke(cache, "two")
        assert not rejected["success"] and rejected["errno"] == 28, rejected
        assert rejected["cache"]["artifact"] == first["cache"]["artifact"]
        if partial:
            assert rejected["cache"]["sequence"] == 2, "failure did not reach target storage"
        release(paths)
        retry = invoke(cache, "latest")
        assert retry["success"] and retry["cache"]["artifact"]["sequence"] == 2
        return {"scenario": "cache_" + kind + ("_target" if partial else ""), "passed": True,
                "fault": fault, "rejected_by": rejected["error"], "old_artifact_preserved": True,
                "metadata_floor": rejected["cache"]["sequence"], "retry": True}


def token_case(kind):
    setup()
    original = CONFIG.read_bytes()
    systemctl("stop", "nlup-lab-agent.service")
    with volume() as mountpoint:
        credentials = mountpoint / "agent"
        directory(credentials, "nlup-agent", "nlup-agent", 0o700)
        atomic_bytes(credentials / "agent.toml", original)
        own(credentials / "agent.toml", "nlup-agent", 0o600, "nlup-agent")
        run("mount", "--bind", str(credentials), str(CONFIG.parent))
        try:
            assert os.stat(CONFIG.parent).st_dev == os.stat(mountpoint).st_dev
            systemctl("start", "nlup-lab-agent.service")
            online(OLD)
            before_pid = probe()["pid"]
            fault, paths = exhaust(mountpoint, kind)
            try:
                response = api("/api/nodes/cold-node/refresh-token", {})
            except LabApiError:
                response = {"ok": False}
            assert not response.get("ok"), "server acknowledged a non-durable token"
            assert CONFIG.read_bytes() == original, "failed write replaced saved credentials"
            online(OLD)
            assert probe()["pid"] == before_pid, "failed token write killed the Agent"
            release(paths)
            response = api("/api/nodes/cold-node/refresh-token", {})
            assert response["ok"] and CONFIG.read_bytes() != original
            run("runuser", "-u", "nlup-agent", "--", str(LIB / "credential-check"),
                "--check-credentials", "--config", str(CONFIG))
            online(OLD)
            return {"scenario": "token_" + kind, "passed": True, "fault": fault,
                    "false_ack_prevented": True, "old_credentials_preserved": True,
                    "process_preserved": True, "retry_persisted": True, "credential_check": True}
        finally:
            # Never expose the stale underlying token after removing the bind mount.
            systemctl("stop", "nlup-lab-agent.service")
            latest = CONFIG.read_bytes()
            assert os.stat(CONFIG.parent).st_dev == os.stat(mountpoint).st_dev
            run("umount", str(CONFIG.parent))
            atomic_bytes(CONFIG, latest)
            own(CONFIG, "nlup-agent", 0o600, "nlup-agent")


def main():
    guard()
    parser = argparse.ArgumentParser()
    parser.add_argument("case", choices=("cache_disk", "cache_inode", "cache_disk_target", "token_disk", "token_inode"))
    args = parser.parse_args()
    setup_mirror()
    try:
        category, kind, *extra = args.case.split("_")
        result = cache_case(kind, bool(extra)) if category == "cache" else token_case(kind)
        result["guest_root_capacity"] = capacity("/")
    finally:
        stop_all()
        systemctl("stop", "nlup-lab-resilience-mirror.service")
        systemctl("disable", "nlup-lab-resilience-mirror.service")
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
