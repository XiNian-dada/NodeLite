#!/usr/bin/env python3
"""Keep incomplete Forgejo release uploads private as drafts."""

import argparse
import hashlib
import json
import os
import re
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class Client:
    def __init__(self, server, repository, token):
        url = urllib.parse.urlsplit(server)
        if (url.scheme != "https" or not url.hostname or url.username or url.password
                or url.query or url.fragment or url.path not in ("", "/")):
            raise ValueError("expected credential-free Forgejo HTTPS origin")
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
            raise ValueError("expected owner/repository")
        self.base = f"{server.rstrip('/')}/api/v1/repos/{repository}"
        self.token = token
        self.opener = urllib.request.build_opener(NoRedirect())

    def request(self, method, path, data=None, content_type="application/json"):
        body = json.dumps(data).encode() if data is not None and content_type == "application/json" else data
        request = urllib.request.Request(self.base + path, data=body, method=method, headers={
            "Authorization": f"token {self.token}", "Content-Type": content_type,
            "Accept": "application/json", "User-Agent": "NodeLite-Release",
        })
        with self.opener.open(request, timeout=180) as response:
            return json.load(response)

    def upload(self, path, file):
        if file.stat().st_size > 128 * 1024 * 1024:
            raise ValueError("release asset exceeds the upload memory budget")
        boundary = uuid.uuid4().hex
        body = (f'--{boundary}\r\nContent-Disposition: form-data; name="attachment"; '
                f'filename="{file.name}"\r\nContent-Type: application/octet-stream\r\n\r\n').encode()
        body += file.read_bytes() + f"\r\n--{boundary}--\r\n".encode()
        return self.request("POST", path, body, f"multipart/form-data; boundary={boundary}")


def check_assets(directory, tag, commit, repository, server):
    targets = ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"]
    names = {f"nodelite-{binary}-{target}" for binary in ["server", "agent", "updater"] for target in targets}
    names |= {f"nodelite-agent-{target}" for target in ["x86_64-apple-darwin", "aarch64-apple-darwin"]}
    names |= {"install-server.sh", "install-server-config.sh", "install-server-upgrade.sh",
              "install-agent.sh", "install-agent-signed.sh", "THIRD_PARTY_NOTICES.md", "BUILD_INFO.json"}
    names |= {f"nodelite-{binary}.cdx.json" for binary in ["server", "agent", "proto", "updater"]}
    if any(not p.is_file() or p.is_symlink() for p in directory.iterdir()):
        raise ValueError("release directory must contain only regular files")
    if {p.name for p in directory.iterdir()} != names | {"SHA256SUMS.txt"}:
        raise ValueError("release directory does not contain the exact release asset set")
    hashes = {}
    for line in (directory / "SHA256SUMS.txt").read_text().splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9_.-]+)", line)
        if not match or match[2] in hashes:
            raise ValueError("invalid or duplicate checksum entry")
        hashes[match[2]] = match[1]
    if set(hashes) != names:
        raise ValueError("checksum manifest must cover every release asset exactly once")
    for name, expected in hashes.items():
        with (directory / name).open("rb") as stream:
            if hashlib.file_digest(stream, "sha256").hexdigest() != expected:
                raise ValueError(f"checksum mismatch: {name}")
    info = json.loads((directory / "BUILD_INFO.json").read_text())
    expected = {"commit": commit, "tag": tag, "repository": repository,
                "server": server, "signed_attestation": False}
    if info != expected:
        raise ValueError("build source record does not match this release")
    return sorted(names | {"SHA256SUMS.txt"})


def publish(client, directory, tag, commit, repository, server):
    if not re.fullmatch(r"v?[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", tag):
        raise ValueError("expected a version tag")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("expected full source commit SHA")
    names = check_assets(directory, tag, commit, repository, server)
    encoded_tag = urllib.parse.quote(tag, safe="")
    if client.request("GET", "/commits/" + encoded_tag)["sha"] != commit:
        raise ValueError("release tag no longer points at the built commit")
    try:
        release = client.request("GET", "/releases/tags/" + encoded_tag)
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        release = client.request("POST", "/releases", {
            "tag_name": tag, "target_commitish": commit, "name": f"NodeLite {tag}",
            "draft": True, "prerelease": "-" in tag,
            "body": f"Source commit: `{commit}`\n\nLinux x86_64/aarch64 musl binaries, macOS Intel/ARM64 Agents, installers, SBOM and SHA256SUMS.txt.\n"
                    "BUILD_INFO.json is a build source record, not a signed attestation.\n"
                    "Default GitHub installer/update URLs and signed Agent enrollment remain separate.",
        })
    if not release["draft"] or release["target_commitish"] != commit:
        raise ValueError("refusing to replace a published release or a draft for another commit")
    path = f"/releases/{release['id']}"
    if client.request("GET", path + "/assets"):
        raise ValueError("draft contains partial uploads; inspect it before retrying")
    for name in names:
        client.upload(path + "/assets?name=" + urllib.parse.quote(name, safe=""), directory / name)
    assets = client.request("GET", path + "/assets")
    uploaded = {asset["name"]: asset["size"] for asset in assets}
    if len(assets) != len(names) or uploaded != {name: (directory / name).stat().st_size for name in names}:
        raise ValueError("uploaded assets do not match the complete release")
    result = client.request("PATCH", path, {"draft": False})
    print(result["html_url"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--tag", required=True)
    args = parser.parse_args()
    try:
        token = os.environ["FORGEJO_TOKEN"]
        server = os.environ["FORGEJO_SERVER"]
        repository = os.environ["FORGEJO_REPOSITORY"]
        commit = os.environ["FORGEJO_COMMIT"]
        if not token:
            raise ValueError("FORGEJO_TOKEN is required")
        publish(Client(server, repository, token), args.directory, args.tag, commit, repository, server)
    except urllib.error.HTTPError as error:
        raise SystemExit(f"Forgejo release API returned HTTP {error.code}; any draft is preserved")
    except (KeyError, ValueError, OSError) as error:
        raise SystemExit(str(error))
