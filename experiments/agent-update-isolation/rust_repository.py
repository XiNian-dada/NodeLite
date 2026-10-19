"""Signed public variants for the real Rust client; all private keys stay in memory."""

from pathlib import Path
import shutil

from tuf_repo import Repository

TARGET = "nodelite-agent-stable-x86_64-unknown-linux-musl"


def prepare(root, binaries, fixture):
    repo = Repository(root / "publisher", TARGET)
    variants = root / "variants"
    variants.mkdir()
    old, new = ((binaries / name).read_bytes() for name in ("agent-old", "agent-new"))

    def publish(name, payload=new, sequence=2, version=2, **kwargs):
        custom = {"product": "nodelite", "component": "agent", "channel": "stable",
                  "target": "x86_64-unknown-linux-musl", "release_sequence": sequence,
                  "min_rollback_sequence": 1, "version": "rust-new"}
        custom.update(kwargs.pop("custom", {}))
        repo.publish(sequence, payload, version=version, custom=custom, **kwargs)
        shutil.copytree(repo.directory, variants / name)

    publish("good")
    publish("tampered")
    for path in (variants / "tampered/targets").iterdir():
        if path.is_file():
            path.write_bytes(b"tampered")
    publish("expired", expires=-60)
    publish("wrong_scope", custom={"product": "not-nodelite"})
    publish("one_signer", target_keys=repo.keys["targets"][:1])
    publish("downgrade", old, sequence=1, version=3)
    publish("equivocation", old, version=3)
    publish("latest", version=4)
    for mode in ("crash", "hang", "preflight_fail"):
        payload = fixture + ('\nNLUP_FIXTURE={"sequence":2,"mode":"' + mode + '"}').encode()
        publish(mode, payload)
    revoked = repo.keys["targets"]
    old_root = repo.keys["root"]
    repo.rotate()
    for name, old_count, new_count in (("root_one_old", 1, 2), ("root_one_new", 2, 1)):
        repo.sign(repo.root, old_root[:old_count] + repo.keys["root"][:new_count])
        repo.write("2.root.json", repo.root)
        publish(name, version=5)
    repo.sign(repo.root, old_root[:2] + repo.keys["root"][:2])
    repo.write("2.root.json", repo.root)
    publish("revoked", target_keys=revoked, version=5)
    publish("revoked_without_root", target_keys=revoked, version=6)
    (variants / "revoked_without_root/metadata/2.root.json").unlink()
    publish("rotated_good", version=7)
    (root / "bootstrap.json").write_bytes(repo.bootstrap)
