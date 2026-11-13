#!/usr/bin/env python3
"""Reject platform regressions before scheduling Forgejo jobs."""

import re
import subprocess
from pathlib import Path

import yaml

root = Path(__file__).resolve().parent.parent
workflows = root / ".forgejo/workflows"
expected = {"ci.yml", "coverage.yml", "release.yml", "pages.yml", "macos.yml"}
assert {p.name for p in workflows.glob("*.yml")} == expected
assert not list((root / ".github/workflows").glob("*.yml")), "legacy workflows would run twice"

for path in sorted(workflows.glob("*.yml")):
    data = yaml.load(path.read_text(), Loader=yaml.BaseLoader)
    assert "on" in data and data["jobs"], path
    jobs = data["jobs"]
    for job_id, job in jobs.items():
        runner = job["runs-on"]
        expected_runner = {"systemd-acceptance": "nodelite-systemd", "agent-macos-native": "macos-x64"}.get(job_id, "mainland")
        assert runner == expected_runner, (path, job_id, "unavailable Runner")
        assert int(job["timeout-minutes"]) <= 120
        needs = job.get("needs", [])
        if isinstance(needs, str):
            needs = [needs]
        assert all(n in jobs for n in needs), (path, job_id, needs)
        image = job.get("container", {}).get("image")
        if image:
            expected_image = "nodelite-macos-cross:20261113-v1" if job_id in {"agent-macos-cross", "build-agent-macos"} else "nodelite-ci:20261113-v2"
            assert image == expected_image, image
        assert not job.get("container", {}).get("options"), "Runner controls container capabilities"
        names = [step.get("name", "") for step in job["steps"]]
        if any("pnpm " in step.get("run", "") for step in job["steps"]):
            assert "Check provisioned build tools" in names, (path, job_id)
            assert image == "nodelite-ci:20261113-v2"
        assert "Fetch exact source archive" in names, (path, job_id)
        archive = next(step for step in job["steps"] if step.get("name") == "Fetch exact source archive")
        assert archive["env"]["SOURCE_COMMIT"] == "${{ forgejo.sha }}"
        assert "--max-time 180" in archive["run"]
        for step in job["steps"]:
            uses = step.get("uses", "")
            if uses:
                assert re.fullmatch(r"https://data\.forgejo\.org/actions/[a-z-]+@[0-9a-f]{40}", uses), uses
                if "/upload-artifact@" in uses:
                    assert uses.endswith("c6a3b2bd78b3985e4b2f15397fec357f0fd808de")
                if "/download-artifact@" in uses:
                    assert uses.endswith("ad191675b41f6a5b46da9a048cb6893812da158b")
                if "/checkout@" in uses:
                    assert step["with"]["persist-credentials"] == "false"
            script = step.get("run")
            if script:
                # Substitute expression placeholders only for shell parsing, never execution.
                script = re.sub(r"\$\{\{.*?\}\}", "placeholder", script)
                result = subprocess.run(["bash", "-n"], input=script, text=True, capture_output=True)
                assert result.returncode == 0, (path, step.get("name"), result.stderr)
    assert "github." not in path.read_text() and "--privileged" not in path.read_text(), path
release = (workflows / "release.yml").read_text()
assert 'if [ "$SOURCE_REF" != "refs/tags/$RELEASE_TAG" ]' in release
assert "Publish complete Forgejo draft" in release and "publish-forgejo-release.py" in release
assert "cargo zigbuild" in release and not re.search(r"(?m)^\s*cross build", release)
assert "cargo-zigbuild --version 0.24.1 --locked" in release
pages = (workflows / "pages.yml").read_text()
assert "NODELITE_ONLINE_PRIVATE_KEY" not in pages and "schedule:" not in pages
assert "deploy-pages" not in pages and 'test ! -d "$RUNNER_TEMP/nodelite-site/updates"' in pages
print("Forgejo workflow dependencies, pinned Actions, shell syntax and publishing boundaries passed")

ci = yaml.load((workflows / "ci.yml").read_text(), Loader=yaml.BaseLoader)
release_jobs = yaml.load(release, Loader=yaml.BaseLoader)["jobs"]
assert "build-agent-macos" in release_jobs["release"]["needs"]
assert ci["jobs"]["agent-macos-cross"]["runs-on"] == "mainland"
for target in ["x86_64-apple-darwin", "aarch64-apple-darwin"]:
    assert target in (root / "scripts/ci/build-macos-agent.sh").read_text()
    assert f"release-agent-macos/nodelite-agent-{target}" in release

macos = yaml.load((workflows / "macos.yml").read_text(), Loader=yaml.BaseLoader)
assert set(macos["on"]) == {"push", "workflow_dispatch"}
assert macos["on"]["push"]["branches"] == ["main"]
assert set(macos["jobs"]) == {"agent-macos-native"}
native = macos["jobs"]["agent-macos-native"]
assert native["runs-on"] == "macos-x64" and "container" not in native
assert int(native["timeout-minutes"]) <= 45
assert any(step.get("if") == "always()" and "cargo +1.93.1 clean" in step.get("run", "") for step in native["steps"])
