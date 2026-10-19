"""A long offline root-renewal chain, generated with disposable operator approvals."""

import hashlib
import json
import os
import secrets
import shutil

from vm import WORK


def root_gap(ceremony):
    original_root, original_roots = ceremony.root, ceremony.roots
    ceremony.roots = ceremony.private / "gap-roots"
    ceremony.roots.mkdir(mode=0o700)
    shutil.copyfile(original_root, ceremony.roots / "1.root.json")
    for version in range(2, 12):
        policy = ceremony.private / ("gap-policy-" + secrets.token_hex(8))
        policy.write_text(json.dumps({"roles": {role: {"threshold": 2 if role in {"root", "targets"} else 1,
            "keys": [name + ".pub.json" for name in names]} for role, names in ceremony.roles.items()}}))
        draft = ceremony.private / f"gap-{version}.unsigned"
        ceremony.cli("root-draft", "--policy", policy, "--previous", ceremony.root, "--out", draft)
        output = ceremony.roots / f"{version}.root.json"
        ceremony.assemble(draft, ceremony.roles["root"][:2], output, previous=ceremony.root)
        ceremony.root = output
    ceremony.release("root-gap", 2, 20, WORK / "rust-binaries/agent-new")
    ceremony.root, ceremony.roots = original_root, original_roots


def operations(ceremony):
    mirror = ceremony.public / "operations"
    shutil.copytree(ceremony.public / "refresh", mirror, symlinks=True)
    for version in (3, 4):
        draft, timestamp = ceremony.private / f"ops-{version}.unsigned", ceremony.private / f"ops-{version}.json"
        ceremony.cli("role-draft", "--role", "timestamp", "--root", ceremony.root,
                     "--child", mirror / "current/metadata/1.snapshot.json", "--metadata-version", version, "--out", draft)
        ceremony.assemble(draft, ceremony.roles["timestamp"], timestamp, authority=ceremony.root)
        ceremony.cli("publish", "--roots", ceremony.roots, "--targets", mirror / "current/metadata/1.targets.json",
            "--snapshot", mirror / "current/metadata/1.snapshot.json", "--timestamp", timestamp,
            "--artifact", WORK / "rust-binaries/agent-old", "--destination", mirror)
    pin = hashlib.sha256(ceremony.root.read_bytes()).hexdigest()
    before = ceremony.cli("check", "--destination", mirror, "--root-sha256", pin)
    assert before["ready"] and before["generations"] == 4
    for path in (mirror / "generations").iterdir():
        os.utime(path, (1, 1))
    archive = ceremony.private / "archive"
    archive.mkdir(mode=0o700)
    flags = ("collect", "--destination", mirror, "--archive", archive, "--keep", 2)
    plan = ceremony.cli(*flags)
    assert len(plan["retired"]) == 2 and not list(archive.iterdir())
    result = ceremony.cli(*flags, "--apply")
    after = ceremony.cli("check", "--destination", mirror, "--root-sha256", pin)
    assert after["ready"] and after["generations"] == 2 and after["current"] == before["current"]
    assert len(list(archive.iterdir())) == 2 and not result["deleted"]
    return {"scenario": "security_public_operations", "passed": True, "real_signed_artifact": True,
            "dry_run_and_atomic_archive": True, "current_trust_and_history_preserved": True}
