"""Dry-run by default; all mutation requires the local administrator's --apply."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import sys

from . import bundle, transaction
from .common import MIGRATION, PreflightError, inspect_legacy, root_guard


def main():
    parser = argparse.ArgumentParser(description="Explicit local-root migration to the signed Agent updater")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--apply", action="store_true")
    mode.add_argument("--resume", action="store_true")
    mode.add_argument("--recover", action="store_true")
    mode.add_argument("--audit", action="store_true")
    mode.add_argument("--upgrade", action="store_true")
    mode.add_argument("--enroll", action="store_true")
    parser.add_argument("--bundle", type=Path)
    parser.add_argument("--bundle-sha256")
    parser.add_argument("--root-sha256")
    parser.add_argument("--metadata-url")
    parser.add_argument("--targets-url")
    parser.add_argument("--health-timeout", type=int, default=60)
    parser.add_argument("--target")
    parser.add_argument("--enable-web-updates", action="store_true")
    args = parser.parse_args()
    if args.enable_web_updates and not (args.enroll or args.upgrade):
        raise ValueError("web update opt-in requires verified enrollment or a local upgrade")
    root_guard()
    if args.recover and not (MIGRATION / "journal.json").exists():
        print(json.dumps({"recovery": "not_pending"}))
        return
    fd = os.open("/run/nodelite-updater-migration.lock", os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.upgrade:
            from .installer import upgrade
            result = upgrade()
        elif args.enroll:
            from .installer import enroll
            if args.bundle is None or args.target is None:
                raise ValueError("installer requires a verified archive and architecture")
            result = enroll(args.bundle, args.target)
        elif args.audit:
            from .audit import check
            result = check()
        elif args.recover:
            result = transaction.recover()
        else:
            if any(value is None for value in (args.bundle, args.bundle_sha256, args.root_sha256, args.metadata_url, args.targets_url)):
                raise ValueError("provide bundle, independent fingerprints and explicit repository URLs")
            if not 10 <= args.health_timeout <= 300:
                raise ValueError("health timeout must be within 10..300 seconds")
            if args.apply or args.resume:
                action = transaction.resume if args.resume else transaction.apply
                result = action(args.bundle, args.bundle_sha256, args.root_sha256,
                                args.metadata_url, args.targets_url, args.health_timeout)
            else:
                manifest, files = bundle.inspect(args.bundle, args.bundle_sha256, args.root_sha256)
                bundle.policy(manifest, files, args.metadata_url, args.targets_url)
                inspect_legacy()
                result = {"plan_only": True, "channel": manifest["channel"], "target": manifest["target"],
                          "baseline_sequence": manifest["baseline_sequence"], "service_changed": False}
        if args.enable_web_updates:
            from .installer import enable_web_updates
            result["web_updates_enabled"] = enable_web_updates()
        print(json.dumps(result))
    finally:
        os.close(fd)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        detail = str(error) if isinstance(error, PreflightError) else type(error).__name__
        print("migration rejected: " + detail + "; inspect protected local state before retrying", file=sys.stderr)
        sys.exit(1)
