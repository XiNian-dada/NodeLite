#!/usr/bin/env python3
"""Reject platform regressions before scheduling Forgejo jobs."""

import re
import subprocess
from pathlib import Path

import yaml

root = Path(__file__).resolve().parent.parent
workflows = root / ".forgejo/workflows"
expected = {"ci.yml", "coverage.yml", "release.yml", "pages.yml"}
assert {p.name for p in workflows.glob("*.yml")} == expected
assert not list((root / ".github/workflows").glob("*.yml")), "legacy workflows would run twice"
for path in sorted(workflows.glob("*.yml")):
    data = yaml.load(path.read_text(), Loader=yaml.BaseLoader)
    assert "on" in data and data["jobs"], path
    jobs = data["jobs"]
    for job_id, job in jobs.items():
        runner = job["runs-on"]
        if runner != "mainland":
            assert job.get("if") and job_id in {"agent-macos-build", "systemd-acceptance"}, (path, job_id)
        assert int(job["timeout-minutes"]) <= 120
        needs = job.get("needs", [])
        if isinstance(needs, str):
            needs = [needs]
        assert all(n in jobs for n in needs), (path, job_id, needs)
        image = job.get("container", {}).get("image")
        if image:
            assert re.search(r"@sha256:[0-9a-f]{64}$", image), image
        names = [step.get("name", "") for step in job["steps"]]
        if "Install pinned pnpm" in names:
            assert names.index("Set up Node.js") < names.index("Install pinned pnpm")
            assert names.index("Install pinned pnpm") < min(i for i, step in enumerate(job["steps"]) if "pnpm install --frozen-lockfile" in step.get("run", ""))
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
