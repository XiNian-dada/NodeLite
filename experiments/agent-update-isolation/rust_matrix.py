#!/usr/bin/env python3
"""Actual signed Rust CLI rejection, activation, rollback and VM power-cut matrix."""

import hashlib
import json
import os
from pathlib import Path

from rust_checks import POINTS
from rust_vm import HERE, guest, prepare, sources
from vm import WORK, boot, stop


def main():
    if Path.cwd().resolve() != WORK.parents[1].resolve() or os.geteuid() == 0:
        raise RuntimeError("run as compiler from ~/NodeLite only")
    output = WORK / "rust-matrix-evidence.json"
    output.unlink(missing_ok=True)
    prepare()
    results = []

    def record(result):
        results.append(result)
        output.write_text(json.dumps({"complete": False, "scenarios": results}, indent=2) + "\n")
        print(json.dumps(result), flush=True)

    try:
        guest("enroll")
        record(guest("activate"))
        for name in ("permissions", "tampered", "expired", "wrong_scope", "one_signer", "root_one_old", "root_one_new", "root_503", "slow_timestamp",
                     "preflight_fail", "crash", "hang", "trial_token_rollback", "failed_download_downgrade",
                     "root_revocation", "corrupt_journal", "storage_disk", "storage_inode", "storage_target"):
            record(guest("case", name))
        for point in POINTS:
            armed = guest("arm", point)
            stop(WORK / "vm-v1")
            boot(WORK / "vm-v1")
            record({"armed": armed, **guest("after", point)})
    except Exception:
        print(json.dumps(guest("diagnose")), flush=True)
        raise
    finally:
        cleanup = guest("halt")
    assert len(results) == 27 and all(result["passed"] for result in results)
    source = WORK.parents[1] / "nodelite-updater"
    paths = [*source.glob("src/*.rs"), *source.glob("systemd/*")]
    build = json.loads((WORK / "rust-binaries/build.json").read_text())
    assert sources() == build["source_sha256"], "sources changed while experiments were running"
    output.write_text(json.dumps({"complete": True, "scenarios": results, "cleanup": cleanup,
        "source_sha256": build["source_sha256"], "build": build,
        "binaries": {name: hashlib.sha256((WORK / "rust-binaries" / name).read_bytes()).hexdigest()
                     for name in ("agent-old", "agent-new", "nodelite-updater", "nodelite-updater-faults")}}, indent=2) + "\n")


if __name__ == "__main__":
    main()
