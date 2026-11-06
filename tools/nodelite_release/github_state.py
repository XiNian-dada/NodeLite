"""Public metadata persisted in Git; release bytes are reconstructed outside Git history."""

from pathlib import Path
import re

from tuf.api.metadata import Metadata

from . import metadata, repository
from .io import atomic, create, decode, digest, directory, read

SCOPES = {f"{channel}/{arch}" for channel in ("stable", "testing") for arch in metadata.ARCHES}


def config(path):
    value = decode(read(path, 65536))
    if set(value) != {"schema", "repositories", "enrollments"} or value["schema"] != 1:
        raise ValueError("unsupported GitHub update configuration")
    scopes = set()
    for entry in value["repositories"]:
        if set(entry) != {"channel", "target", "root_sha256"}:
            raise ValueError("unexpected repository configuration")
        scope = entry["channel"] + "/" + entry["target"]
        if scope not in SCOPES or scope in scopes or not re.fullmatch(r"[0-9a-f]{64}", entry["root_sha256"]):
            raise ValueError("invalid or duplicate update scope")
        scopes.add(scope)
    tags = set()
    for entry in value["enrollments"]:
        if set(entry) != {"tag", "sha256"} or not valid_tag(entry["tag"]) \
                or not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]) or entry["tag"] in tags:
            raise ValueError("invalid enrollment approval")
        tags.add(entry["tag"])
    if len(tags) > 32 or len(scopes) > 4:
        raise ValueError("hosting inventory requires explicit maintenance")
    return value


def valid_tag(tag):
    return isinstance(tag, str) and re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?", tag)


def scope(entry):
    return entry["channel"] + "/" + entry["target"]


def inventory(path):
    repository.owned_directory(path)
    result, total = {}, 0
    for item in Path(path).iterdir():
        if not repository.META_NAME.fullmatch(item.name) or len(result) >= 8192:
            raise ValueError("unexpected metadata file or retention limit")
        raw = read(item)
        total += len(raw)
        if total > 64 * 1024 * 1024:
            raise ValueError("metadata storage budget exceeded")
        document = Metadata.from_dict(decode(raw))
        limit = {"root": 262144, "snapshot": 262144, "timestamp": 16384, "targets": 2 * 1024 * 1024}.get(document.signed.type, 0)
        if len(raw) > limit:
            raise ValueError("metadata exceeds Agent runtime limit")
        expected = "timestamp.json" if document.signed.type == "timestamp" else \
            f"{document.signed.version}.{document.signed.type}.json"
        if expected != item.name:
            raise ValueError("metadata name and signed version differ")
        result[item.name] = raw
    return result


def inspect(path, entry, *, online=True, fresh=False):
    values = inventory(path)
    chain = repository.roots(path)
    if not any(digest(values[f"{root.signed.version}.root.json"]) == entry["root_sha256"] for root in chain):
        raise ValueError("metadata does not extend approved bootstrap root")
    targets, locators = [], {}
    for name, raw in values.items():
        document = Metadata.from_bytes(raw)
        if document.signed.type == "root":
            continue
        # Historical roles may be signed by a key removed during a later root rotation.
        accepted = False
        for root in chain:
            try:
                root.verify_delegate(document.signed.type, document)
                accepted = True
                break
            except Exception:
                continue
        if not accepted:
            raise ValueError("invalid historical metadata signature")
        if document.signed.type == "targets":
            if len(document.signed.targets) != 1:
                raise ValueError("one target per update scope required")
            target_name, info = next(iter(document.signed.targets.items()))
            custom = info.unrecognized_fields.get("custom", {})
            if metadata.release_scope(custom) != target_name or scope(custom) != scope(entry) \
                    or set(info.hashes) != {"sha256"} or not 0 < info.length <= repository.LIMIT \
                    or not re.fullmatch(r"[0-9a-f]{64}", info.hashes["sha256"]) \
                    or not valid_tag("v" + custom["version"]) \
                    or custom["channel"] == "stable" and "-" in custom["version"]:
                raise ValueError("signed target scope or release tag rejected")
            locator = custom["version"]
            if locator in locators and locators[locator] != info.hashes["sha256"]:
                raise ValueError("historical Release asset bytes changed")
            locators[locator] = info.hashes["sha256"]
            targets.append((document, name, custom, info))
        elif online is False:
            raise ValueError("offline approvals must contain only roots and targets")
    if not targets:
        raise ValueError("missing approved targets")
    targets.sort(key=lambda item: item[0].signed.version)
    for old, new in zip(targets, targets[1:]):
        advance_target(old[2], old[3], new[2], new[3])
    if online:
        timestamp = metadata.load(Path(path) / "timestamp.json")
        snapshot_path = Path(path) / f"{timestamp.signed.snapshot_meta.version}.snapshot.json"
        snapshot = metadata.load(snapshot_path)
        target_path = Path(path) / f"{snapshot.signed.meta['targets.json'].version}.targets.json"
        documents, custom, info, _ = repository.release(
            chain[-1], target_path, snapshot_path, Path(path) / "timestamp.json", fresh=fresh)
        if documents[0].signed.version != targets[-1][0].signed.version:
            raise ValueError("state pointer does not select newest approved targets")
    else:
        documents = [targets[-1][0]]
        custom, info = targets[-1][2:]
    if fresh and chain[-1].signed.expires <= metadata.now():
        raise ValueError("bootstrap root requires offline renewal")
    return values, chain, documents, custom, info, targets


def advance_target(old, old_info, new, new_info):
    if scope(old) != scope(new) or new["release_sequence"] < old["release_sequence"] \
            or new["min_rollback_sequence"] < old["min_rollback_sequence"]:
        raise ValueError("target scope, sequence or floor rollback")
    if new["release_sequence"] == old["release_sequence"] and (new != old or new_info != old_info):
        raise ValueError("same release sequence has different bytes or policy")


def folders(base, entry):
    path = Path(base)
    for name in (*scope(entry).split("/"), "metadata"):
        child = path / name
        if not child.exists():
            directory(child, 0o755)
        repository.owned_directory(child)
        path = child
    return path


def prepare(configuration, approvals, previous, output, *, initialize=False):
    if not configuration["repositories"]:
        raise ValueError("updates enabled without independently approved repositories")
    directory(output, 0o755)
    for entry in configuration["repositories"]:
        relative = scope(entry) + "/metadata"
        approved = inspect(Path(approvals) / relative, entry, online=False, fresh=True)
        values = dict(approved[0])
        old_path = Path(previous) / relative
        if old_path.exists():
            old = inspect(old_path, entry)
            if len(approved[1]) < len(old[1]) or approved[2][0].signed.version < old[2][0].signed.version:
                raise ValueError("offline approvals would roll back published state")
            advance_target(old[3], old[4], approved[3], approved[4])
            for name, raw in old[0].items():
                if name in values and values[name] != raw:
                    raise ValueError("immutable metadata equivocation")
                values[name] = raw
        elif not initialize:
            raise ValueError("missing durable state; explicit initialization required")
        if approved[2][0].signed.expires <= metadata.now():
            raise ValueError("targets requires offline renewal")
        target = folders(output, entry)
        for name, raw in values.items():
            create(target / name, raw)


def verify_transition(previous, candidate, entry):
    new = inspect(candidate, entry, fresh=True)
    if Path(previous).exists():
        old = inspect(previous, entry)
        if len(new[1]) < len(old[1]):
            raise ValueError("root rollback")
        for name, raw in old[0].items():
            if name != "timestamp.json" and new[0].get(name) != raw:
                raise ValueError("immutable metadata was removed or changed")
        advance_target(old[3], old[4], new[3], new[4])
        for before, after in zip(old[2], new[2]):
            if after.signed.version < before.signed.version or after.signed.version == before.signed.version \
                    and after.to_bytes() != before.to_bytes():
                raise ValueError("online metadata rollback or equivocation")
    return new


def save(configuration, public, destination):
    for entry in configuration["repositories"]:
        old = Path(destination) / scope(entry) / "metadata"
        new = Path(public) / scope(entry) / "metadata"
        values = verify_transition(old, new, entry)[0]
        folder = folders(destination, entry)
        for name, raw in values.items():
            path = folder / name
            if not path.exists():
                create(path, raw)
            elif read(path) != raw:
                if name != "timestamp.json":
                    raise ValueError("immutable state collision")
                atomic(path, raw)
