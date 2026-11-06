"""Offline release handoff: verified public assets, enrollment archives and pinned commands."""

from contextlib import ExitStack
from pathlib import Path
import re
import shlex
import tempfile
from urllib.parse import urlsplit

from . import bundle, repository
from .io import LIMIT, create, digest, directory, json_bytes, read

SOURCE = Path(__file__).resolve().parents[2]
LINUX = r"(?:x86_64|aarch64)-unknown-linux-musl"
ASSET = re.compile(
    rf"nodelite-(?:agent|server|updater)-{LINUX}|"
    r"nodelite-agent-(?:x86_64|aarch64)-apple-darwin|"
    r"install-(?:agent(?:-signed)?|server(?:-config|-upgrade)?)\.sh|"
    r"nodelite-(?:proto|agent|server|updater)\.cdx\.json|THIRD_PARTY_NOTICES\.md")


def assets(path):
    """Snapshot a flat release directory; the manifest is integrity, not independent trust."""
    path = Path(path)
    checksums = {}
    for line in read(path / "SHA256SUMS.txt").decode("ascii").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9._-]+)", line)
        if not match or not ASSET.fullmatch(match[2]) or match[2] in checksums:
            raise ValueError("unexpected or duplicate release checksum entry")
        checksums[match[2]] = match[1]
    names = {p.name for p in path.iterdir()}
    if names - {"SHA256SUMS.txt", "provenance.intoto.jsonl"} != checksums.keys():
        raise ValueError("release inventory differs from checksums")
    result = {}
    for name, expected in checksums.items():
        payload = read(path / name, 128 * 1024 * 1024)
        if digest(payload) != expected:
            raise ValueError("release asset checksum mismatch")
        result[name] = payload
    # The CI provenance file is generated after its checksum manifest; preserve it separately.
    if "provenance.intoto.jsonl" in names:
        result["provenance.intoto.jsonl"] = read(path / "provenance.intoto.jsonl", 16 * 1024 * 1024)
    for name in ("install-agent.sh", "install-agent-signed.sh"):
        if result.get(name) != read(SOURCE / "scripts" / name):
            raise ValueError("use the reviewed release checkout matching both installer assets")
    return result


def command(tag, base_url, pins):
    """The outer installer is verified before sudo, independently of remote SHA256SUMS."""
    q = shlex.quote
    return f"""#!/bin/sh
# Obtain this file and its pins through your independently trusted release approval channel.
# Run on an existing default-path Linux Agent. Pass --bootstrap-url for a fresh installation.
set -eu
umask 077
stage=$(mktemp -d)
trap 'rm -f "$stage/install-agent.sh"; rmdir "$stage"' EXIT HUP INT TERM
curl -q -fsSL --connect-timeout 15 --max-time 60 --max-filesize 1048576 \\
  --proto '=https,http' --proto-redir '=https' {q(base_url + '/install-agent.sh')} -o "$stage/install-agent.sh"
printf '%s  %s\\n' {q(pins['installer_sha256'])} "$stage/install-agent.sh" | sha256sum -c -
sudo env NODELITE_AGENT_VERSION={q(tag)} sh "$stage/install-agent.sh" \\
  --base-url {q(base_url)} --enable-web-updates \\
  --enrollment-sha256 {q(pins['archive_sha256'])} \\
  --signed-helper-sha256 {q(pins['helper_sha256'])} "$@"
"""


def stage(repositories, anchors, metadata_urls, targets_urls, release_assets, units, tag, output,
          release_url=None, channel="stable"):
    if not re.fullmatch(r"v?[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?", tag):
        raise ValueError("explicit release tag required")
    if not 1 <= len(repositories) <= 2 or not all(
            len(items) == len(repositories) for items in (anchors, metadata_urls, targets_urls)):
        raise ValueError("each repository needs one trusted root pin and two fixed URLs")
    release_url = release_url or f"https://github.com/XiNian-dada/NodeLite/releases/download/{tag}"
    parsed = urlsplit(release_url)
    if parsed.scheme not in {"http", "https"} or not parsed.hostname or parsed.username is not None \
            or parsed.password is not None or parsed.query or parsed.fragment \
            or parsed.scheme == "http" and parsed.hostname not in {"127.0.0.1", "::1"} \
            or any(ord(char) < 33 for char in release_url) or '"' in release_url or "\\" in release_url:
        raise ValueError("release URL requires HTTPS or isolated loopback")
    payloads = assets(release_assets)
    approved = []
    # Hold every repository stable until the corresponding pack has been checked.
    with ExitStack() as locks, tempfile.TemporaryDirectory() as temp:
        temporary = Path(temp)
        for index, path in enumerate(repositories):
            locks.enter_context(repository.locked(path, shared=True))
            generation, chain, _, custom, info = repository.current(path, fresh=True)
            if not re.fullmatch(r"[0-9a-f]{64}", anchors[index]) or not any(
                    digest(read(generation / "metadata" / f"{root.signed.version}.root.json")) == anchors[index]
                    for root in chain):
                raise ValueError("repository does not extend the independently approved root")
            target = custom["target"]
            if custom["version"] != tag.removeprefix("v") or custom["channel"] != channel \
                    or any(item["target"] == target for item in approved):
                raise ValueError("release version, channel or architecture mismatch")
            agent, updater = (payloads.get(f"nodelite-{name}-{target}") for name in ("agent", "updater"))
            if agent is None or updater is None:
                raise ValueError("matching release Agent and updater are required")
            info.verify_length_and_hashes(agent)
            staged_updater = temporary / f"updater-{index}"
            create(staged_updater, updater)
            archive_name = f"nodelite-enrollment-{target}.tar.gz"
            archive = temporary / archive_name
            pins = bundle.pack_installer(path, staged_updater, units, archive, metadata_urls[index], targets_urls[index])
            if pins["baseline_sha256"] != digest(agent) or pins["target"] != target:
                raise ValueError("enrollment baseline differs from approved release")
            payloads[archive_name] = read(archive, 2 * LIMIT + 1024 * 1024)
            pins.update(installer_sha256=digest(payloads["install-agent.sh"]),
                        helper_sha256=digest(payloads["install-agent-signed.sh"]),
                        tag=tag, channel=channel, metadata_url=metadata_urls[index],
                        targets_url=targets_urls[index], archive=archive_name)
            approved.append(pins)
        # All validation precedes creation; existing outputs are never overwritten or merged blindly.
        output = directory(output)
        public = directory(output / "assets")
        for name, payload in sorted(payloads.items()):
            create(public / name, payload)
        manifest = "".join(f"{digest(data)}  {name}\n" for name, data in sorted(payloads.items()))
        create(public / "SHA256SUMS.txt", manifest.encode())
        for pins in approved:
            create(output / f"enable-web-updates-{pins['target']}.sh", command(tag, release_url, pins).encode())
        result = {"schema": 1, "tag": tag, "release_url": release_url, "enrollments": approved,
                  "checksums_sha256": digest(manifest.encode()), "private_keys_included": False}
        create(output / "approval.json", json_bytes(result))
        return result
