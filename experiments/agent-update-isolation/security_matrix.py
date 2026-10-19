"""Separate probe evidence from final acceptance; only the marked enrollment KVM guest."""

import argparse
import json
import os
from pathlib import Path

from enrollment_matrix import PROJECT, VM, guest, source_hashes
from vm import WORK, boot, stop


def main():
    if Path.cwd().resolve() != PROJECT.resolve() or os.geteuid() == 0:
        raise RuntimeError("compiler ~/NodeLite only")
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe", action="store_true")
    args = parser.parse_args()
    source = source_hashes()
    expected = json.loads((WORK / "enrollment-build.json").read_text())
    if not args.probe:
        assert expected["source_sha256"] == source
    results = []
    try:
        results.append(guest("case", "security_root_gap"))
        if not args.probe:
            operations = json.loads((WORK / "enrollment-input/fingerprints.json").read_text())["operations"]
            results.append(operations)
            for name in ("security_resume_policy", "security_missing_trust", "security_updater_lock", "security_root_rejected",
                         "security_legacy_installer", "security_node_audit"):
                results.append(guest("case", name))
        results.append(guest("arm", "security_switching_backup"))
        stop(VM)
        boot(VM)
        results.append(guest("after", "security_switching_backup"))
        if not args.probe:
            results.append(guest("arm", "security_committed_boot"))
            stop(VM)
            boot(VM)
            results.append(guest("after", "security_committed_boot"))
    finally:
        cleanup = guest("halt")
    assert source_hashes() == source
    accepted = all(item.get("passed", True) for item in results)
    if not args.probe:
        assert sum("passed" in item for item in results) == 10
    evidence = {"complete": not args.probe and accepted, "probe": args.probe, "scenarios": results,
                "cleanup": cleanup, "build": expected, "current_source_sha256": source,
                "build_snapshot_matches": expected["source_sha256"] == source}
    name = "security-probe.json" if args.probe else "security-evidence.json"
    (WORK / name).write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence), flush=True)
    if not args.probe and not accepted:
        raise RuntimeError("security acceptance failed")


if __name__ == "__main__":
    main()
