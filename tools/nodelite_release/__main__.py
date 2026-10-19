"""Human-controlled root ceremonies and independent detached role signing."""

import argparse
import json
from pathlib import Path
import sys

from . import bundle, keys, metadata, repository


def parser():
    command = argparse.ArgumentParser(description="NodeLite offline signing and static TUF publishing")
    sub = command.add_subparsers(dest="action", required=True)
    key = sub.add_parser("keygen")
    key.add_argument("--private", type=Path, required=True)
    key.add_argument("--public", type=Path, required=True)
    key.add_argument("--password-fd", type=int)
    root = sub.add_parser("root-draft")
    root.add_argument("--policy", type=Path, required=True)
    root.add_argument("--previous", type=Path)
    root.add_argument("--out", type=Path, required=True)
    sign = sub.add_parser("approve")
    sign.add_argument("--metadata", type=Path, required=True)
    sign.add_argument("--private", type=Path, required=True)
    sign.add_argument("--password-fd", type=int)
    sign.add_argument("--out", type=Path, required=True)
    assemble = sub.add_parser("assemble")
    assemble.add_argument("--unsigned", type=Path, required=True)
    assemble.add_argument("--approval", type=Path, action="append", required=True)
    assemble.add_argument("--root", type=Path)
    assemble.add_argument("--previous", type=Path)
    assemble.add_argument("--out", type=Path, required=True)
    target = sub.add_parser("targets-draft")
    for name in ("artifact", "out"):
        target.add_argument("--" + name, type=Path, required=True)
    for name in ("channel", "target", "version"):
        target.add_argument("--" + name, required=True)
    for name in ("sequence", "floor", "metadata-version"):
        target.add_argument("--" + name, type=int, required=True)
    role = sub.add_parser("role-draft")
    role.add_argument("--role", choices=("snapshot", "timestamp"), required=True)
    role.add_argument("--metadata-version", type=int)
    for name in ("child", "root", "out"):
        role.add_argument("--" + name, type=Path, required=True)
    publish = sub.add_parser("publish")
    for name in ("roots", "targets", "snapshot", "timestamp", "artifact", "destination"):
        publish.add_argument("--" + name, type=Path, required=True)
    pack = sub.add_parser("pack-enrollment")
    pack.add_argument("--bootstrap-version", type=int)
    for name in ("repository", "updater", "units", "out"):
        pack.add_argument("--" + name, type=Path, required=True)
    return command


def main():
    args = parser().parse_args()
    result = None
    if args.action == "keygen":
        result = keys.generate(args.private, args.public, keys.password(args.password_fd, confirm=True))
    elif args.action == "root-draft":
        metadata.root_draft(args.policy, args.out, args.previous)
    elif args.action == "approve":
        result = keys.approve(args.metadata, args.private, args.out, keys.password(args.password_fd))
    elif args.action == "assemble":
        metadata.assemble(args.unsigned, args.approval, args.root, args.out, args.previous)
    elif args.action == "targets-draft":
        metadata.target_draft(args.artifact, args.channel, args.target, args.version, args.sequence,
                              args.floor, args.metadata_version, args.out)
    elif args.action == "role-draft":
        metadata.role_draft(args.role, args.child, args.root, args.out, args.metadata_version)
    elif args.action == "publish":
        result = repository.publish(args.roots, args.targets, args.snapshot, args.timestamp, args.artifact, args.destination)
    elif args.action == "pack-enrollment":
        result = bundle.pack(args.repository, args.updater, args.units, args.out, args.bootstrap_version)
    print(json.dumps(result or {"created": str(args.out)}))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # Library exceptions may carry operator input; role/key failures need no secret detail.
        print("release operation rejected: " + type(error).__name__, file=sys.stderr)
        sys.exit(1)
