"""One short-lived Actions job signs exactly one online role; no offline key is accepted."""

from datetime import timedelta
import os
from pathlib import Path

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from securesystemslib.signer import CryptoSigner
from tuf.api.metadata import Metadata, MetaFile, Snapshot, Timestamp

from . import github_state as state, metadata
from .io import atomic, create, read


def signer_from_environment():
    # This is the only CI credential bridge. Secrets never enter subprocesses or files.
    private = os.environ.pop("NODELITE_ONLINE_PRIVATE_KEY", "").encode()
    password = os.environ.pop("NODELITE_ONLINE_PASSWORD", "").encode()
    if len(private) > 16384 or not 12 <= len(password) <= 1024:
        raise ValueError("missing or invalid online signing credentials")
    key = serialization.load_pem_private_key(private, password=password)
    if not isinstance(key, Ed25519PrivateKey):
        raise ValueError("online signer requires encrypted Ed25519")
    return CryptoSigner(key)


def renew(configuration, candidate, role, signer):
    if role not in {"snapshot", "timestamp"}:
        raise ValueError("CI cannot sign root or targets")
    results = []
    for entry in configuration["repositories"]:
        path = Path(candidate) / state.scope(entry) / "metadata"
        values, chain, _, _, _, targets = state.inspect(path, entry, online=None, fresh=True)
        root = chain[-1]
        if signer.public_key.keyid not in root.signed.roles[role].keyids:
            raise ValueError("credential is not authorized for the requested online role")
        target = targets[-1]
        metadata.verified(target[0], root)
        if role == "snapshot":
            version = max((int(name.split(".")[0]) for name in values if name.endswith(".snapshot.json")), default=0) + 1
            child = MetaFile.from_data(target[0].signed.version, values[target[1]], ["sha256"])
            document = Metadata(Snapshot(version=version, expires=metadata.now() + timedelta(days=7),
                                         meta={"targets.json": child}))
            output = path / f"{version}.snapshot.json"
        else:
            number = max(int(name.split(".")[0]) for name in values if name.endswith(".snapshot.json"))
            raw = read(path / f"{number}.snapshot.json")
            snapshot = Metadata.from_bytes(raw)
            metadata.verified(snapshot, root)
            if set(snapshot.signed.meta) != {"targets.json"} \
                    or snapshot.signed.meta["targets.json"].version != target[0].signed.version:
                raise ValueError("snapshot does not select approved targets")
            snapshot.signed.meta["targets.json"].verify_length_and_hashes(values[target[1]])
            old = metadata.load(path / "timestamp.json") if "timestamp.json" in values else None
            version = old.signed.version + 1 if old else 1
            child = MetaFile.from_data(number, raw, ["sha256"])
            document = Metadata(Timestamp(version=version, expires=metadata.now() + timedelta(hours=12), snapshot_meta=child))
            output = path / "timestamp.json"
        if version >= 2**53:
            raise ValueError("metadata version limit")
        document.sign(signer)
        root.verify_delegate(role, document)
        (atomic if output.exists() else create)(output, document.to_bytes())
        results.append({"scope": state.scope(entry), "role": role, "version": version})
    return results
