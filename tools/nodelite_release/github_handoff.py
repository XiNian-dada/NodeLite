"""Pin a complete offline enrollment handoff without replacing original release checksums."""

import gzip
import io
from pathlib import Path
import re
import tarfile

from . import web
from .io import create, decode, digest, directory, read

LIMIT = 384 * 1024 * 1024
ARCH = r"(?:x86_64|aarch64)-unknown-linux-musl"


def allowed(name):
    if name in {"approval.json", "assets/SHA256SUMS.txt", "assets/provenance.intoto.jsonl"}:
        return True
    if re.fullmatch(rf"enable-web-updates-{ARCH}\.sh", name):
        return True
    if not name.startswith("assets/"):
        return False
    return web.ASSET.fullmatch(name[7:]) or re.fullmatch(rf"nodelite-enrollment-{ARCH}\.tar\.gz", name[7:])


def verify(files):
    approval = decode(files["approval.json"])
    checksums = files["assets/SHA256SUMS.txt"]
    if digest(checksums) != approval["checksums_sha256"] or approval["private_keys_included"] is not False:
        raise ValueError("handoff checksum approval mismatch")
    expected = {"approval.json", "assets/SHA256SUMS.txt"}
    for line in checksums.decode("ascii").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9._-]+)", line)
        if not match:
            raise ValueError("invalid handoff checksums")
        name = "assets/" + match[2]
        if name in expected or not allowed(name) or digest(files[name]) != match[1]:
            raise ValueError("handoff inventory or digest mismatch")
        expected.add(name)
    for pins in approval["enrollments"]:
        name = f"enable-web-updates-{pins['target']}.sh"
        if name in expected or files[name] != web.command(approval["tag"], approval["release_url"], pins).encode() \
                or digest(files["assets/" + pins["archive"]]) != pins["archive_sha256"]:
            raise ValueError("enable command differs from approved pins")
        expected.add(name)
    if expected != files.keys() or not approval["enrollments"]:
        raise ValueError("unexpected handoff files")
    return approval


def pack(source, output):
    files, total = {}, 0
    for path in Path(source).rglob("*"):
        if path.is_symlink():
            raise ValueError("handoff symlinks rejected")
        if path.is_dir():
            if path.relative_to(source).as_posix() != "assets":
                raise ValueError("unexpected handoff directory")
            continue
        name = path.relative_to(source).as_posix()
        if not allowed(name):
            raise ValueError("unexpected handoff file")
        files[name] = read(path, 128 * 1024 * 1024)
        total += len(files[name])
        if total > LIMIT:
            raise ValueError("handoff exceeds hosting budget")
    verify(files)
    buffer = io.BytesIO()
    with gzip.GzipFile(fileobj=buffer, mode="wb", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            for name, raw in sorted(files.items()):
                entry = tarfile.TarInfo(name)
                entry.size, entry.mode = len(raw), 0o644
                archive.addfile(entry, io.BytesIO(raw))
    payload = buffer.getvalue()
    create(output, payload)
    return {"sha256": digest(payload), "bytes": len(payload)}


def unpack(path, expected_hash, output, *, tag, base_url, repositories, trusted_roots=None):
    raw = read(path, LIMIT)
    if digest(raw) != expected_hash:
        raise ValueError("handoff differs from independently approved archive")
    files, total = {}, 0
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as archive:
        for member in archive:
            if not member.isfile() or not allowed(member.name) or member.name in files \
                    or not 0 <= member.size <= 128 * 1024 * 1024 or len(files) >= 32:
                raise ValueError("unsafe handoff archive member")
            total += member.size
            if total > LIMIT:
                raise ValueError("expanded handoff exceeds budget")
            files[member.name] = archive.extractfile(member).read(member.size + 1)
    approval = verify(files)
    if approval["tag"] != tag or approval["release_url"] != base_url + f"/updates/enrollment/{tag}/assets":
        raise ValueError("handoff tag or Pages asset URL mismatch")
    for pins in approval["enrollments"]:
        scope = pins["channel"] + "/" + pins["target"]
        entry = next((item for item in repositories if item["channel"] + "/" + item["target"] == scope), None)
        roots = (trusted_roots or {}).get(scope, [entry["root_sha256"]] if entry else [])
        if entry is None or pins["root_sha256"] not in roots \
                or pins["metadata_url"] != base_url + "/updates/" + scope + "/metadata/" \
                or pins["targets_url"] != base_url + "/updates/" + scope + "/targets/":
            raise ValueError("handoff does not match configured trust and endpoints")
    directory(output, 0o755)
    directory(Path(output) / "assets", 0o755)
    for name, payload in files.items():
        create(Path(output) / name, payload)
    return approval
