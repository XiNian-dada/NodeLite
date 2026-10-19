"""Exercise operator CLI ceremonies with disposable encrypted keys, never production keys."""

import json
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile

from vm import WORK

PROJECT = WORK.parents[1]


def build(output):
    output.mkdir(mode=0o700)
    public = output / "public"
    public.mkdir(mode=0o755)
    with tempfile.TemporaryDirectory(prefix="offline-signers-", dir=WORK) as temporary:
        private = Path(temporary)
        secret = secrets.token_urlsafe(32).encode()

        def cli(*args, signing=False, success=True):
            command = [sys.executable, "-m", "tools.nodelite_release", *map(str, args)]
            if signing:
                command += ["--password-fd", "0"]
            result = subprocess.run(command, input=secret + b"\n" if signing else None, capture_output=True, timeout=90, cwd=PROJECT)
            if success and result.returncode:
                raise RuntimeError(result.stderr.decode()[-1000:])
            if not success:
                assert result.returncode != 0
                return
            return json.loads(result.stdout)

        roles = {}
        for role, count in (("root", 3), ("targets", 2), ("snapshot", 1), ("timestamp", 1)):
            roles[role] = []
            for index in range(count):
                name = f"{role}-{index}"
                cli("keygen", "--private", private / (name + ".pem"), "--public", private / (name + ".pub.json"), signing=True)
                roles[role].append(name)
        roots = private / "roots"
        roots.mkdir(mode=0o700)

        def ceremony(previous=None, old=None):
            policy = private / ("policy-" + secrets.token_hex(4) + ".json")
            policy.write_text(json.dumps({"roles": {role: {"threshold": 2 if role in {"root", "targets"} else 1,
                "keys": [name + ".pub.json" for name in names]} for role, names in roles.items()}}))
            draft = private / ("root-" + secrets.token_hex(4) + ".unsigned")
            cli("root-draft", "--policy", policy, "--out", draft, *(["--previous", previous] if previous else []))
            output_root = roots / ("2.root.json" if previous else "1.root.json")
            assemble(draft, (old or [])[:2] + roles["root"][:2], output_root, previous)
            return output_root

        def assemble(draft, names, target, previous=None, authority=None):
            approvals = []
            for name in names:
                approval = private / ("approval-" + secrets.token_hex(8) + ".json")
                cli("approve", "--metadata", draft, "--private", private / (name + ".pem"), "--out", approval, signing=True)
                approvals += ["--approval", approval]
            cli("assemble", "--unsigned", draft, *approvals, "--out", target,
                *(["--root", authority] if authority else []),
                *(["--previous", previous] if previous else []))

        root = ceremony()

        def release(name, sequence, version, artifact, target_authority=None):
            work = private / (name + "-roles")
            work.mkdir(mode=0o700)
            targets, snapshot, timestamp = (work / (role + ".json") for role in ("targets", "snapshot", "timestamp"))
            draft = work / "targets.unsigned"
            cli("targets-draft", "--artifact", artifact, "--channel", "stable", "--target", "x86_64-unknown-linux-musl",
                "--version", "rust-old" if sequence == 1 else "rust-new", "--sequence", sequence, "--floor", 1,
                "--metadata-version", version, "--out", draft)
            assemble(draft, roles["targets"], targets, authority=target_authority or root)
            for role, child, target in (("snapshot", targets, snapshot), ("timestamp", snapshot, timestamp)):
                draft = work / (role + ".unsigned")
                cli("role-draft", "--role", role, "--root", target_authority or root, "--child", child, "--out", draft)
                assemble(draft, roles[role], target, authority=root)
            mirror = public / name
            args = ("publish", "--roots", roots, "--targets", targets, "--snapshot", snapshot, "--timestamp", timestamp,
                    "--artifact", artifact, "--destination", mirror)
            if target_authority is None:
                cli(*args)
            else:
                cli(*args, success=False)
                # A malicious mirror can serve old-key metadata even though the publisher refuses it.
                generation = mirror / "generations" / secrets.token_hex(16)
                (generation / "metadata").mkdir(parents=True)
                (generation / "targets").mkdir()
                for path in roots.glob("*.json"):
                    shutil.copyfile(path, generation / "metadata" / path.name)
                for role, path in (("targets", targets), ("snapshot", snapshot), ("timestamp", timestamp)):
                    filename = "timestamp.json" if role == "timestamp" else f"{version}.{role}.json"
                    shutil.copyfile(path, generation / "metadata" / filename)
                import hashlib
                filename = hashlib.sha256(artifact.read_bytes()).hexdigest() + ".nodelite-agent-stable-x86_64-unknown-linux-musl"
                shutil.copyfile(artifact, generation / "targets" / filename)
                (mirror / "current").symlink_to("generations/" + generation.name)
            return mirror, (targets, snapshot, timestamp)

        base, paths = release("base", 1, 1, WORK / "rust-binaries/agent-old")
        info = cli("pack-enrollment", "--repository", base, "--updater", PROJECT / "target/release/nodelite-updater",
                   "--units", PROJECT / "nodelite-updater/systemd", "--out", output / "bundle")
        refresh = public / "refresh"
        shutil.copytree(base, refresh, symlinks=True)
        draft, timestamp = private / "timestamp-refresh.unsigned", private / "timestamp-refresh.json"
        cli("role-draft", "--role", "timestamp", "--root", root, "--child", paths[1], "--metadata-version", 2, "--out", draft)
        assemble(draft, roles["timestamp"], timestamp, authority=root)
        cli("publish", "--roots", roots, "--targets", paths[0], "--snapshot", paths[1], "--timestamp", timestamp,
            "--artifact", WORK / "rust-binaries/agent-old", "--destination", refresh)
        old_root, old_targets = root, roles["targets"]
        old = roles["root"]
        for role, count in (("root", 3), ("targets", 2)):
            roles[role] = []
            for index in range(count):
                name = f"rotated-{role}-{index}"
                cli("keygen", "--private", private / (name + ".pem"), "--public", private / (name + ".pub.json"), signing=True)
                roles[role].append(name)
        root = ceremony(root, old)
        rotated, _ = release("rotated", 2, 3, WORK / "rust-binaries/agent-new")
        info["next"] = cli("pack-enrollment", "--repository", rotated, "--updater", PROJECT / "target/release/nodelite-updater",
            "--units", PROJECT / "nodelite-updater/systemd", "--bootstrap-version", 1, "--out", output / "bundle-next")
        current_targets = roles["targets"]
        roles["targets"] = old_targets
        release("revoked", 2, 4, WORK / "rust-binaries/agent-new", target_authority=old_root)
        roles["targets"] = current_targets
        release("rotated-recovery", 2, 5, WORK / "rust-binaries/agent-new")
        for path in public.rglob("*"):
            if path.is_file():
                assert ".pem" not in path.name
                if path.name.endswith(".json"):
                    assert b"PRIVATE KEY" not in path.read_bytes()
    # Only public inputs survive the context; passphrases never enter the guest or evidence.
    (output / "fingerprints.json").write_text(json.dumps(info) + "\n")
    return info
