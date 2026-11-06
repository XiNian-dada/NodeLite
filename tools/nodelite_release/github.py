"""GitHub-hosted publishing CLI: durable metadata, isolated renewal and bounded Pages export."""

import argparse
import html
import json
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlsplit

from . import github_handoff, github_online, github_state as state
from .io import LIMIT, create, digest, directory, read
from .metadata import release_scope


def download(repository, tag, asset, cache, *, limit=LIMIT):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository) or not state.valid_tag(tag) \
            or not re.fullmatch(r"[A-Za-z0-9_.-]+", asset):
        raise ValueError("invalid GitHub release locator")
    target = Path(cache) / tag
    if not target.exists():
        directory(target)
    state.repository.owned_directory(target)
    output = target / asset
    if not output.exists():
        # Redirects occur only in the publisher; signed target hashes remain authoritative.
        url = f"https://github.com/{repository}/releases/download/{tag}/{asset}"
        subprocess.run(["curl", "-q", "--fail", "--silent", "--show-error", "--location",
                        "--proto", "=https", "--proto-redir", "=https", "--connect-timeout", "15",
                        "--max-time", "180", "--max-filesize", str(limit), "--output", str(output), url],
                       check=True, timeout=190)
    return output


def endpoint(value):
    value = value.rstrip("/")
    parsed = urlsplit(value)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username is not None \
            or parsed.password is not None or parsed.query or parsed.fragment \
            or any(ord(char) < 33 for char in value) or "%" in value or "\\" in value:
        raise ValueError("Pages base URL requires plain HTTPS")
    return value


def render(configuration, previous, candidate, output, cache, repository, base_url, *, initialize=False):
    base_url = endpoint(base_url)
    directory(output, 0o755)
    index, used, trusted_roots = [], 0, {}
    configured = {state.scope(entry) for entry in configuration["repositories"]}
    if any((Path(previous) / scope).exists() and scope not in configured for scope in state.SCOPES):
        raise ValueError("published scopes cannot silently disappear")
    for entry in configuration["repositories"]:
        relative = state.scope(entry) + "/metadata"
        old = Path(previous) / relative
        if not old.exists() and not initialize:
            raise ValueError("missing durable state requires explicit initialization")
        values, _, documents, custom, _, targets = state.verify_transition(old, Path(candidate) / relative, entry)
        trusted_roots[state.scope(entry)] = [digest(raw) for name, raw in values.items() if name.endswith(".root.json")]
        folder = state.folders(output, entry)
        target_folder = directory(folder.parent / "targets", 0o755)
        for name, raw in values.items():
            create(folder / name, raw)
            used += len(raw)
        for _, _, signed, info in targets:
            name = info.hashes["sha256"] + "." + release_scope(signed)
            if (target_folder / name).exists():
                continue
            asset = download(repository, "v" + signed["version"], "nodelite-agent-" + signed["target"], cache, limit=info.length)
            raw = read(asset, LIMIT)
            info.verify_length_and_hashes(raw)
            used += len(raw)
            budget(used)
            create(target_folder / name, raw)
        index.append({"scope": state.scope(entry), "version": custom["version"],
                      "expires": documents[-1].signed.expires.isoformat(), "root_sha256": entry["root_sha256"]})
    enrollment_folder = directory(Path(output) / "enrollment", 0o755)
    links = []
    for approval in configuration["enrollments"]:
        path = download(repository, approval["tag"], "nodelite-web-enrollment.tar.gz", cache, limit=github_handoff.LIMIT)
        destination = enrollment_folder / approval["tag"]
        result = github_handoff.unpack(path, approval["sha256"], destination, tag=approval["tag"],
                                       base_url=base_url, repositories=configuration["repositories"], trusted_roots=trusted_roots)
        for file in destination.rglob("*"):
            if file.is_file():
                used += file.stat().st_size
        budget(used)
        for item in result["enrollments"]:
            links.append((approval["tag"], item["target"]))
    create(Path(output) / "status.json", (json.dumps({"repositories": index, "public_bytes": used}, indent=2) + "\n").encode())
    publish_index(output, index, links)
    return {"repositories": index, "public_bytes": used, "enrollments": len(links)}


def budget(size):
    if size > 900 * 1024 * 1024:
        raise ValueError("Pages update assets exceed 900 MiB; retain history and plan explicit maintenance")


def publish_index(output, entries, links):
    text = '<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>NodeLite Agent 更新源</title>'
    text += '<h1>NodeLite Agent 更新源</h1><p>文件由 GitHub 托管；更新器按本机固定公钥验签。</p>'
    text += '<p>首次启用前，请从独立可信的发布审批记录核对脚本和根指纹。</p><ul>'
    for entry in entries:
        text += '<li>' + html.escape(entry["scope"] + ": " + entry["version"] + "；有效期至 " + entry["expires"]) + '</li>'
    text += '</ul><h2>首次启用脚本</h2><ul>'
    for tag, target in links:
        path = f"enrollment/{tag}/enable-web-updates-{target}.sh"
        text += '<li><a href="' + html.escape(path, quote=True) + '">' + html.escape(tag + " / " + target) + '</a></li>'
    text += '</ul></html>'
    create(Path(output) / "index.html", text.encode())


def parser():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    prepare = actions.add_parser("prepare")
    prepare.add_argument("--approvals", type=Path, required=True)
    prepare.add_argument("--previous", type=Path, required=True)
    prepare.add_argument("--out", type=Path, required=True)
    prepare.add_argument("--initialize", action="store_true")
    renew = actions.add_parser("renew")
    renew.add_argument("--candidate", type=Path, required=True)
    renew.add_argument("--role", choices=("snapshot", "timestamp"), required=True)
    build = actions.add_parser("render")
    for name in ("previous", "candidate", "out", "cache"):
        build.add_argument("--" + name, type=Path, required=True)
    build.add_argument("--repository", required=True)
    build.add_argument("--base-url", required=True)
    build.add_argument("--initialize", action="store_true")
    save = actions.add_parser("save-state")
    save.add_argument("--public", type=Path, required=True)
    save.add_argument("--destination", type=Path, required=True)
    for command in (prepare, renew, build, save):
        command.add_argument("--config", type=Path, required=True)
    pack = actions.add_parser("pack-handoff")
    pack.add_argument("--source", type=Path, required=True)
    pack.add_argument("--out", type=Path, required=True)
    return parser


def main():
    args = parser().parse_args()
    if args.action == "pack-handoff":
        result = github_handoff.pack(args.source, args.out)
    else:
        configuration = state.config(args.config)
        if args.action == "prepare":
            result = state.prepare(configuration, args.approvals, args.previous, args.out, initialize=args.initialize)
        elif args.action == "renew":
            result = github_online.renew(configuration, args.candidate, args.role, github_online.signer_from_environment())
        elif args.action == "save-state":
            result = state.save(configuration, args.public, args.destination)
        else:
            result = render(configuration, args.previous, args.candidate, args.out, args.cache,
                            args.repository, args.base_url, initialize=args.initialize)
    print(json.dumps(result or {"prepared": True}))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print("GitHub publication rejected: " + type(error).__name__, file=sys.stderr)
        sys.exit(1)
