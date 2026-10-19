#!/usr/bin/env python3
"""Fetch fixed official artifacts for lab tests, not for production installation."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from vm import WORK

TARGET = "x86_64-unknown-linux-musl"
VERSIONS = ("3.0.3", "3.0.4")
RELEASES = WORK / "official-releases"


def fetch(url, destination):
    fd, temporary = tempfile.mkstemp(dir=destination.parent, prefix=".download-")
    os.close(fd)
    try:
        subprocess.run(["curl", "--fail", "--location", "--silent", "--show-error",
                        "--proto", "=https", "--proto-redir", "=https", "--max-time", "180",
                        "--retry", "2", "--output", temporary, url], check=True)
        os.replace(temporary, destination)
    finally:
        Path(temporary).unlink(missing_ok=True)


def main():
    if Path.cwd().resolve() != WORK.parents[1].resolve() or os.geteuid() == 0:
        raise RuntimeError("run as the unprivileged compiler from ~/NodeLite only")
    evidence = []
    for version in VERSIONS:
        directory = RELEASES / version
        directory.mkdir(parents=True, mode=0o700, exist_ok=True)
        base = f"https://github.com/XiNian-dada/NodeLite/releases/download/v{version}"
        fetch(base + "/SHA256SUMS.txt", directory / "SHA256SUMS.txt")
        sums = dict((Path(line.split()[1].lstrip("*")).name, line.split()[0])
                    for line in (directory / "SHA256SUMS.txt").read_text().splitlines())
        for component in ("agent", "server"):
            name = f"nodelite-{component}-{TARGET}"
            path = directory / name
            fetch(base + "/" + name, path)
            actual = hashlib.sha256(path.read_bytes()).hexdigest()
            if actual != sums[name]:
                raise RuntimeError("official release checksum mismatch")
            path.chmod(0o500)
            # Older Agent CLI has no --version: verify the reported version after authentication.
            subprocess.run([str(path), "--help"], check=True, capture_output=True, text=True)
            evidence.append({"version": version, "component": component, "url": base + "/" + name,
                             "sha256": actual, "size": path.stat().st_size})
    (RELEASES / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence), flush=True)


if __name__ == "__main__":
    main()
