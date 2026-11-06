"""Release handoff checks use real threshold signatures and independently supplied pins."""

from pathlib import Path
import subprocess

from . import metadata, repository, web
from .io import create, decode, digest, directory, read
from .test_release import _CeremonyFixture

TARGET = "x86_64-unknown-linux-musl"


class WebRelease(_CeremonyFixture):
    def setUp(self):
        super().setUp()
        self.publish()
        self.assets = directory(self.path / "assets")
        self.payloads = {"install-agent.sh": read(web.SOURCE / "scripts/install-agent.sh"),
                         "install-agent-signed.sh": read(web.SOURCE / "scripts/install-agent-signed.sh"),
                         f"nodelite-agent-{TARGET}": read(self.artifact),
                         f"nodelite-updater-{TARGET}": b"\x7fELFnormal-updater",
                         "nodelite-agent-aarch64-apple-darwin": b"other platform retained"}
        for name, data in self.payloads.items():
            create(self.assets / name, data)
        self.checksums()
        self.output = self.path / "handoff"

    def checksums(self):
        (self.assets / "SHA256SUMS.txt").unlink(missing_ok=True)
        create(self.assets / "SHA256SUMS.txt", "".join(
            f"{digest(data)}  {name}\n" for name, data in self.payloads.items()).encode())

    def stage(self, **changes):
        args = dict(repositories=[self.path / "mirror"], anchors=[digest(read(self.root))],
                    metadata_urls=["https://updates.example.invalid/metadata/"],
                    targets_urls=["https://updates.example.invalid/targets/"],
                    release_assets=self.assets, units=web.SOURCE / "nodelite-updater/systemd",
                    tag="v3.0.5", output=self.output)
        args.update(changes)
        return web.stage(**args)

    def test_complete_manifest_pins_and_outer_installer_guard(self):
        result = self.stage()
        public = self.output / "assets"
        manifest = dict((line.split()[1], line.split()[0]) for line in read(public / "SHA256SUMS.txt").decode().splitlines())
        self.assertEqual(set(manifest), {item.name for item in public.iterdir()} - {"SHA256SUMS.txt"})
        for name, expected in manifest.items():
            self.assertEqual(digest(read(public / name, 128 * 1024 * 1024)), expected)
        pins = result["enrollments"][0]
        self.assertEqual(pins["baseline_sha256"], digest(read(self.artifact)))
        script = self.output / f"enable-web-updates-{TARGET}.sh"
        subprocess.run(["sh", "-n", script], check=True)
        text = read(script).decode()
        self.assertLess(text.index("sha256sum -c"), text.index("sudo env"))
        self.assertIn("NODELITE_AGENT_VERSION=v3.0.5", text)
        self.assertIn(pins["archive_sha256"], text)
        self.assertEqual(decode(read(self.output / "approval.json")), result)
        with self.assertRaises(FileExistsError):
            self.stage()

    def test_foreign_version_root_channel_and_url_rejected_before_output(self):
        for changes in ({"tag": "v3.0.6"}, {"anchors": ["0" * 64]}, {"channel": "testing"},
                        {"release_url": "http://remote.invalid/release"},
                        {"metadata_urls": ["https://updates.invalid/?bad"]}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.stage(**changes)
            self.assertFalse(self.output.exists())

    def test_manifest_corruption_duplicate_traversal_and_private_file_rejected(self):
        manifest = self.assets / "SHA256SUMS.txt"
        good = read(manifest)
        for bad in (good + good, b"0" * 64 + b"  ../private.pem\n", good.replace(b"  install-agent.sh", b"  private.pem")):
            manifest.write_bytes(bad)
            with self.assertRaises(ValueError):
                self.stage()
        manifest.write_bytes(good)
        create(self.assets / "private.pem", b"not even read")
        with self.assertRaises(ValueError):
            self.stage()
        self.assertFalse(self.output.exists())

    def test_matching_checksum_does_not_authorize_foreign_baseline_or_helper(self):
        for name in (f"nodelite-agent-{TARGET}", "install-agent-signed.sh"):
            original = self.payloads[name]
            self.payloads[name] = original + b"altered"
            (self.assets / name).write_bytes(self.payloads[name])
            self.checksums()
            with self.assertRaises(Exception):
                self.stage()
            self.assertFalse(self.output.exists())
            self.payloads[name] = original
            (self.assets / name).write_bytes(original)
            self.checksums()

    def test_symlink_and_unmanifested_input_rejected(self):
        name = f"nodelite-agent-{TARGET}"
        (self.assets / name).unlink()
        (self.assets / name).symlink_to(self.artifact)
        with self.assertRaises(OSError):
            self.stage()

    def test_two_architectures_are_staged_together_with_all_original_assets(self):
        target = "aarch64-unknown-linux-musl"
        work = directory(self.path / "arm-roles")
        draft, targets = work / "targets.unsigned", work / "targets.json"
        metadata.target_draft(self.artifact, "stable", target, "3.0.5", 1, 1, 1, draft)
        self.assemble(draft, self.roles["targets"], targets)
        snapshot, timestamp = work / "snapshot.json", work / "timestamp.json"
        for role, child, output in (("snapshot", targets, snapshot), ("timestamp", snapshot, timestamp)):
            draft = work / (role + ".unsigned")
            metadata.role_draft(role, child, self.root, draft)
            self.assemble(draft, self.roles[role], output)
        mirror = self.path / "arm-mirror"
        repository.publish(self.roots, targets, snapshot, timestamp, self.artifact, mirror)
        for kind in ("agent", "updater"):
            name = f"nodelite-{kind}-{target}"
            self.payloads[name] = read(self.artifact)
            create(self.assets / name, self.payloads[name])
        self.checksums()
        result = self.stage(repositories=[self.path / "mirror", mirror],
                            anchors=[digest(read(self.root))] * 2,
                            metadata_urls=[f"https://updates.invalid/{arch}/metadata/" for arch in ("x86", "arm")],
                            targets_urls=[f"https://updates.invalid/{arch}/targets/" for arch in ("x86", "arm")])
        self.assertEqual({p["target"] for p in result["enrollments"]}, {TARGET, target})
        for name in self.payloads:
            self.assertEqual(read(self.output / "assets" / name), self.payloads[name])
