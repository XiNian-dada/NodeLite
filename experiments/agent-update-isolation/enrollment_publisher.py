"""Exercise operator CLI ceremonies with disposable encrypted keys, never production keys."""

import hashlib
import json
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile

from vm import WORK

PROJECT = WORK.parents[1]


class Ceremony:
    def __init__(self, private, public):
        self.private, self.public = private, public
        self.secret = secrets.token_urlsafe(32).encode()
        self.roles = {role: self.keyset(role, count) for role, count in
                      (("root", 3), ("targets", 2), ("snapshot", 1), ("timestamp", 1))}
        self.roots = private / "roots"
        self.roots.mkdir(mode=0o700)
        self.root = self.root_ceremony()

    def cli(self, *args, signing=False, success=True):
        command = [sys.executable, "-m", "tools.nodelite_release", *map(str, args)]
        if signing:
            command += ["--password-fd", "0"]
        result = subprocess.run(command, input=self.secret + b"\n" if signing else None,
                                capture_output=True, timeout=90, cwd=PROJECT)
        if success and result.returncode:
            raise RuntimeError(result.stderr.decode()[-1000:])
        if not success:
            assert result.returncode != 0
            return None
        return json.loads(result.stdout)

    def keyset(self, prefix, count):
        names = [f"{prefix}-{index}" for index in range(count)]
        for name in names:
            self.cli("keygen", "--private", self.private / (name + ".pem"),
                     "--public", self.private / (name + ".pub.json"), signing=True)
        return names

    def assemble(self, draft, names, target, previous=None, authority=None):
        approvals = []
        for name in names:
            approval = self.private / ("approval-" + secrets.token_hex(8) + ".json")
            self.cli("approve", "--metadata", draft, "--private", self.private / (name + ".pem"),
                     "--out", approval, signing=True)
            approvals += ["--approval", approval]
        self.cli("assemble", "--unsigned", draft, *approvals, "--out", target,
                 *(["--root", authority] if authority else []),
                 *(["--previous", previous] if previous else []))

    def root_ceremony(self, previous=None, old=None):
        policy = self.private / ("policy-" + secrets.token_hex(4) + ".json")
        policy.write_text(json.dumps({"roles": {role: {"threshold": 2 if role in {"root", "targets"} else 1,
            "keys": [name + ".pub.json" for name in names]} for role, names in self.roles.items()}}))
        draft = self.private / ("root-" + secrets.token_hex(4) + ".unsigned")
        self.cli("root-draft", "--policy", policy, "--out", draft, *(["--previous", previous] if previous else []))
        target = self.roots / ("2.root.json" if previous else "1.root.json")
        self.assemble(draft, (old or [])[:2] + self.roles["root"][:2], target, previous)
        return target

    def release(self, name, sequence, version, artifact, target_authority=None):
        work = self.private / (name + "-roles")
        work.mkdir(mode=0o700)
        targets, snapshot, timestamp = (work / (role + ".json") for role in ("targets", "snapshot", "timestamp"))
        draft = work / "targets.unsigned"
        self.cli("targets-draft", "--artifact", artifact, "--channel", "stable", "--target", "x86_64-unknown-linux-musl",
            "--version", "rust-old" if sequence == 1 else "rust-new", "--sequence", sequence, "--floor", 1,
            "--metadata-version", version, "--out", draft)
        self.assemble(draft, self.roles["targets"], targets, authority=target_authority or self.root)
        for role, child, target in (("snapshot", targets, snapshot), ("timestamp", snapshot, timestamp)):
            draft = work / (role + ".unsigned")
            self.cli("role-draft", "--role", role, "--root", target_authority or self.root, "--child", child, "--out", draft)
            self.assemble(draft, self.roles[role], target, authority=self.root)
        mirror = self.public / name
        self.cli("publish", "--roots", self.roots, "--targets", targets, "--snapshot", snapshot, "--timestamp", timestamp,
                 "--artifact", artifact, "--destination", mirror, success=target_authority is None)
        if target_authority is not None:
            self.hostile_fixture(mirror, version, artifact, (targets, snapshot, timestamp))
        return mirror, (targets, snapshot, timestamp)

    def hostile_fixture(self, mirror, version, artifact, paths):
        # A malicious mirror can serve old-key metadata even though the publisher refuses it.
        generation = mirror / "generations" / secrets.token_hex(16)
        (generation / "metadata").mkdir(parents=True)
        (generation / "targets").mkdir()
        for path in self.roots.glob("*.json"):
            shutil.copyfile(path, generation / "metadata" / path.name)
        for role, path in zip(("targets", "snapshot", "timestamp"), paths):
            filename = "timestamp.json" if role == "timestamp" else f"{version}.{role}.json"
            shutil.copyfile(path, generation / "metadata" / filename)
        filename = hashlib.sha256(artifact.read_bytes()).hexdigest() + ".nodelite-agent-stable-x86_64-unknown-linux-musl"
        shutil.copyfile(artifact, generation / "targets" / filename)
        (mirror / "current").symlink_to("generations/" + generation.name)

    def refresh(self, base, paths):
        mirror = self.public / "refresh"
        shutil.copytree(base, mirror, symlinks=True)
        draft, timestamp = self.private / "timestamp-refresh.unsigned", self.private / "timestamp-refresh.json"
        self.cli("role-draft", "--role", "timestamp", "--root", self.root, "--child", paths[1],
                 "--metadata-version", 2, "--out", draft)
        self.assemble(draft, self.roles["timestamp"], timestamp, authority=self.root)
        self.cli("publish", "--roots", self.roots, "--targets", paths[0], "--snapshot", paths[1], "--timestamp", timestamp,
                 "--artifact", WORK / "rust-binaries/agent-old", "--destination", mirror)

    def pack(self, repository, output, bootstrap=None):
        return self.cli("pack-enrollment", "--repository", repository, "--updater", PROJECT / "target/release/nodelite-updater",
            "--units", PROJECT / "nodelite-updater/systemd", "--out", output,
            *(["--bootstrap-version", bootstrap] if bootstrap is not None else []))


def build(output):
    output.mkdir(mode=0o700)
    public = output / "public"
    public.mkdir(mode=0o755)
    with tempfile.TemporaryDirectory(prefix="offline-signers-", dir=WORK) as temporary:
        ceremony = Ceremony(Path(temporary), public)
        base, paths = ceremony.release("base", 1, 1, WORK / "rust-binaries/agent-old")
        info = ceremony.pack(base, output / "bundle")
        ceremony.refresh(base, paths)
        from security_publisher import operations, root_gap
        info["operations"] = operations(ceremony)
        root_gap(ceremony)
        old_root, old_targets, old_keys = ceremony.root, ceremony.roles["targets"], ceremony.roles["root"]
        for role, count in (("root", 3), ("targets", 2)):
            ceremony.roles[role] = ceremony.keyset("rotated-" + role, count)
        ceremony.root = ceremony.root_ceremony(old_root, old_keys)
        rotated, _ = ceremony.release("rotated", 2, 3, WORK / "rust-binaries/agent-new")
        info["next"] = ceremony.pack(rotated, output / "bundle-next", bootstrap=1)
        current_targets = ceremony.roles["targets"]
        ceremony.roles["targets"] = old_targets
        ceremony.release("revoked", 2, 4, WORK / "rust-binaries/agent-new", target_authority=old_root)
        ceremony.roles["targets"] = current_targets
        ceremony.release("rotated-recovery", 2, 5, WORK / "rust-binaries/agent-new")
        for path in public.rglob("*"):
            if path.is_file():
                assert ".pem" not in path.name
                if path.name.endswith(".json"):
                    assert b"PRIVATE KEY" not in path.read_bytes()
    # Only public inputs survive the context; passphrases never enter the guest or evidence.
    (output / "fingerprints.json").write_text(json.dumps(info) + "\n")
    return info
