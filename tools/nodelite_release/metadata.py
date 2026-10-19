"""Reference TUF metadata; root ceremonies are separate from online publication."""

from datetime import datetime, timedelta, timezone
from pathlib import Path
import re

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from securesystemslib.signer import SSlibKey
from tuf.api.metadata import Metadata, MetaFile, Root, Snapshot, TargetFile, Targets, Timestamp

from .io import LIMIT, create, decode, read
from .keys import combine

ROLES = {"root", "targets", "snapshot", "timestamp"}
ARCHES = {"x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"}


def now():
    return datetime.now(timezone.utc)


def load(path):
    raw = read(path)
    decode(raw)
    return Metadata.from_bytes(raw)


def root_safe(root):
    if root.type != "root" or set(root.roles) != ROLES or not root.consistent_snapshot:
        raise ValueError("unsupported root roles or snapshot mode")
    seen = set()
    for role, entry in root.roles.items():
        floor = 2 if role in {"root", "targets"} else 1
        if entry.threshold < floor or entry.threshold > len(entry.keyids) \
                or len(set(entry.keyids)) != len(entry.keyids) or seen.intersection(entry.keyids):
            raise ValueError("root threshold, duplicate or cross-role key reuse rejected")
        for keyid in entry.keyids:
            key = root.keys[keyid]
            if key.keytype != "ed25519" or key.scheme != "ed25519":
                raise ValueError("root requires Ed25519 keys")
            canonical = SSlibKey.from_crypto(Ed25519PublicKey.from_public_bytes(bytes.fromhex(key.keyval["public"])))
            if canonical.keyid != keyid:
                raise ValueError("noncanonical key IDs cannot inflate signing thresholds")
        seen.update(entry.keyids)
    if set(root.keys) != seen:
        raise ValueError("unassigned root keys rejected")


def verified(document, root):
    if document.signed.expires <= now():
        raise ValueError("metadata expired")
    root_safe(root.signed)
    root.verify_delegate(document.signed.type, document)


def root_draft(policy, output, previous=None):
    config = decode(read(policy))
    if set(config) != {"roles"} or set(config["roles"]) != ROLES:
        raise ValueError("root policy roles rejected")
    old = load(previous) if previous else None
    root = Root(version=old.signed.version + 1 if old else 1,
                expires=now() + timedelta(days=365), consistent_snapshot=True)
    for role, entry in config["roles"].items():
        if set(entry) != {"threshold", "keys"} or type(entry["threshold"]) is not int:
            raise ValueError("invalid root policy")
        identifiers = set()
        for path in entry["keys"]:
            item = decode(read(Path(policy).parent / path, 16384))
            if set(item) != {"keyid", "key"}:
                raise ValueError("invalid public key document")
            if item["keyid"] in identifiers:
                raise ValueError("duplicate custodian key")
            identifiers.add(item["keyid"])
            root.add_key(SSlibKey.from_dict(item["keyid"], item["key"]), role)
        root.roles[role].threshold = entry["threshold"]
    root_safe(root)
    create(output, Metadata(root).to_bytes())


def assemble(unsigned, approvals, authority, output, previous=None):
    document = combine(load(unsigned), approvals)
    root = document if document.signed.type == "root" else load(authority)
    verified(document, root)
    if document.signed.type == "root":
        if document.signed.version > 1:
            if previous is None:
                raise ValueError("root rotation requires previous root authority")
            old = load(previous)
            root_safe(old.signed)
            if document.signed.version != old.signed.version + 1:
                raise ValueError("root version must advance exactly once")
            old.verify_delegate("root", document)
        elif previous is not None or document.signed.version != 1:
            raise ValueError("invalid initial root ceremony")
    create(output, document.to_bytes())


def release_scope(custom):
    expected = {"product", "component", "channel", "target", "release_sequence", "min_rollback_sequence", "version"}
    if set(custom) != expected or custom["product"] != "nodelite" or custom["component"] != "agent" \
            or custom["channel"] not in {"stable", "testing"} or custom["target"] not in ARCHES \
            or not isinstance(custom["version"], str) or not re.fullmatch(r"[A-Za-z0-9._-]{1,64}", custom["version"]):
        raise ValueError("signed release scope rejected")
    for key in ("release_sequence", "min_rollback_sequence"):
        if type(custom[key]) is not int or not 0 < custom[key] < 2**53:
            raise ValueError("invalid release sequence")
    if custom["min_rollback_sequence"] > custom["release_sequence"]:
        raise ValueError("rollback floor exceeds release")
    return "nodelite-agent-" + custom["channel"] + "-" + custom["target"]


def target_draft(artifact, channel, target, version, sequence, floor, metadata_version, output):
    custom = dict(product="nodelite", component="agent", channel=channel, target=target,
                  version=version, release_sequence=sequence, min_rollback_sequence=floor)
    name = release_scope(custom)
    payload = read(artifact, LIMIT)
    if not payload or payload[:4] != b"\x7fELF":
        raise ValueError("Agent artifact must be a nonempty ELF image")
    info = TargetFile.from_data(name, payload)
    info.unrecognized_fields["custom"] = custom
    if not 0 < metadata_version < 2**53:
        raise ValueError("invalid metadata version")
    create(output, Metadata(Targets(version=metadata_version, expires=now() + timedelta(days=14),
                                   targets={name: info})).to_bytes())


def role_draft(role, child_path, authority, output):
    child = load(child_path)
    verified(child, load(authority))
    if (role, child.signed.type) not in {("snapshot", "targets"), ("timestamp", "snapshot")}:
        raise ValueError("invalid metadata dependency")
    info = MetaFile.from_data(child.signed.version, read(child_path), ["sha256"])
    if role == "snapshot":
        value = Snapshot(version=child.signed.version, expires=now() + timedelta(days=7), meta={"targets.json": info})
    else:
        value = Timestamp(version=child.signed.version, expires=now() + timedelta(hours=12), snapshot_meta=info)
    create(output, Metadata(value).to_bytes())
