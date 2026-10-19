"""Ephemeral test publisher using the TUF reference metadata API, not release tooling."""

from copy import deepcopy
from datetime import datetime, timedelta, timezone
import hashlib
from pathlib import Path

from securesystemslib.signer import CryptoSigner
from tuf.api.metadata import Metadata, MetaFile, Root, Snapshot, TargetFile, Targets, Timestamp

TARGET = "agent/stable/x86_64-unknown-linux-musl/agent"


def expiry(seconds=3600):
    return datetime.now(timezone.utc) + timedelta(seconds=seconds)


class Repository:
    def __init__(self, directory, target=TARGET):
        self.directory = directory
        self.target = target
        (directory / "metadata").mkdir(parents=True)
        (directory / Path("targets") / Path(target).parent).mkdir(parents=True)
        self.keys = {role: [CryptoSigner.generate_ed25519() for _ in range(count)]
                     for role, count in (("root", 3), ("targets", 2), ("snapshot", 1), ("timestamp", 1))}
        root = Root(expires=expiry(86400), consistent_snapshot=True)
        for role, keys in self.keys.items():
            for key in keys:
                root.add_key(key.public_key, role)
            root.roles[role].threshold = 2 if role in {"root", "targets"} else 1
        self.root = Metadata(root)
        self.sign(self.root, self.keys["root"][:2])
        self.write("1.root.json", self.root)
        self.bootstrap = self.root.to_bytes()

    @staticmethod
    def sign(metadata, keys):
        metadata.signatures.clear()
        for key in keys:
            metadata.sign(key, append=True)

    def write(self, name, metadata):
        (self.directory / "metadata" / name).write_bytes(metadata.to_bytes())

    def publish(self, sequence, payload, *, version=None, target_keys=None, expires=3600, custom=None):
        version = version or sequence
        info = TargetFile.from_data(self.target, payload)
        info.unrecognized_fields["custom"] = custom or {
            "product": "nodelite-lab", "component": "agent", "channel": "stable",
            "target": "x86_64-unknown-linux-musl", "release_sequence": sequence}
        self.targets = Metadata(Targets(version=version, expires=expiry(expires), targets={self.target: info}))
        self.sign(self.targets, target_keys or self.keys["targets"])
        target_bytes = self.targets.to_bytes()
        snapshot = Metadata(Snapshot(version=version, expires=expiry(expires), meta={
            "targets.json": MetaFile.from_data(version, target_bytes, ["sha256"])}))
        self.sign(snapshot, self.keys["snapshot"])
        self.timestamp = Metadata(Timestamp(version=version, expires=expiry(expires),
            snapshot_meta=MetaFile.from_data(version, snapshot.to_bytes(), ["sha256"])))
        self.sign(self.timestamp, self.keys["timestamp"])
        self.write(f"{version}.targets.json", self.targets)
        self.write(f"{version}.snapshot.json", snapshot)
        self.write("timestamp.json", self.timestamp)
        prefix = hashlib.sha256(payload).hexdigest()
        target = Path(self.target)
        path = self.directory / "targets" / target.parent / (prefix + "." + target.name)
        path.write_bytes(payload)
        return "/targets/" + str(path.relative_to(self.directory / "targets"))

    def rotate(self, *, old_count=2, new_count=2):
        previous = self.keys["root"]
        root = deepcopy(self.root.signed)
        root.version += 1
        for role in ("root", "targets"):
            for keyid in list(root.roles[role].keyids):
                root.revoke_key(keyid, role)
            self.keys[role] = [CryptoSigner.generate_ed25519() for _ in range(3 if role == "root" else 2)]
            for key in self.keys[role]:
                root.add_key(key.public_key, role)
        self.root = Metadata(root)
        self.sign(self.root, previous[:old_count] + self.keys["root"][:new_count])
        self.write(f"{root.version}.root.json", self.root)
