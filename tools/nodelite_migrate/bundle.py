"""Read a complete bundle snapshot before any root-side publication or execution."""

import re
from pathlib import Path
from urllib.parse import urlsplit

from tools.nodelite_release.io import LIMIT, decode, digest, read

from .common import UNITS


def inspect(path, expected_bundle, expected_root):
    if not all(re.fullmatch(r"[0-9a-f]{64}", value) for value in (expected_bundle, expected_root)):
        raise ValueError("independently supplied trust fingerprints are required")
    path = Path(path)
    manifest_bytes = read(path / "manifest.json", 16384)
    if digest(manifest_bytes) != expected_bundle:
        raise ValueError("bundle fingerprint mismatch")
    manifest = decode(manifest_bytes)
    if set(manifest) != {"schema", "channel", "target", "baseline_sequence", "minimum_sequence", "files"} \
            or manifest["schema"] != 1 or manifest["channel"] not in {"stable", "testing"} \
            or manifest["target"] not in {"x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"}:
        raise ValueError("bundle schema/scope rejected")
    for name in ("baseline_sequence", "minimum_sequence"):
        if type(manifest[name]) is not int or not 0 < manifest[name] < 2**53:
            raise ValueError("bundle release sequence rejected")
    if manifest["minimum_sequence"] > manifest["baseline_sequence"]:
        raise ValueError("baseline below rollback floor")
    expected = {"root.json", "nodelite-agent", "nodelite-updater"} | {"systemd/" + name for name in UNITS}
    if set(manifest["files"]) != expected:
        raise ValueError("bundle cannot contain arbitrary files, scripts or private keys")
    files = {name: read(path / name, LIMIT if name in {"nodelite-agent", "nodelite-updater"} else 262144) for name in expected}
    for name, value in files.items():
        if digest(value) != manifest["files"][name]:
            raise ValueError("bundle content fingerprint mismatch")
    if digest(files["root.json"]) != expected_root:
        raise ValueError("bootstrap root fingerprint mismatch")
    for name in ("nodelite-agent", "nodelite-updater"):
        if files[name][:4] != b"\x7fELF":
            raise ValueError("bundle program is not an ELF")
    if b"/etc/nlup-lab-guest" in files["nodelite-updater"]:
        raise ValueError("lab fault binary cannot be enrolled")
    return manifest, files


def policy(manifest, files, metadata_url, targets_url):
    for value in (metadata_url, targets_url):
        parsed = urlsplit(value)
        if parsed.scheme not in {"http", "https"} or not parsed.hostname or parsed.username is not None \
                or parsed.password is not None or parsed.query or parsed.fragment or not parsed.path.endswith("/") \
                or parsed.scheme == "http" and parsed.hostname not in {"127.0.0.1", "::1"} \
                or any(ord(char) < 33 for char in value) or '"' in value or "\\" in value:
            raise ValueError("repository policy URL rejected")
    return (f'enabled = true\nallow_remote_updates = false\nmetadata_url = "{metadata_url}"\ntargets_url = "{targets_url}"\n'
            f'channel = "{manifest["channel"]}"\ntarget = "{manifest["target"]}"\n'
            f'baseline_sha256 = "{digest(files["nodelite-agent"])}"\n'
            f'baseline_sequence = {manifest["baseline_sequence"]}\nminimum_sequence = {manifest["minimum_sequence"]}\n'
            'trial_seconds = 60\nstability_seconds = 10\n').encode()
