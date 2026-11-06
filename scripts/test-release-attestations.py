#!/usr/bin/env python3
"""Exercise each standalone verifier without downloading or running candidates."""
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parent.parent
with tempfile.TemporaryDirectory(prefix="nodelite-attest-test-") as directory:
    work = Path(directory)
    gh = work / "gh"
    gh.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$GH_ARGS"\nexit "$GH_RESULT"\n')
    gh.chmod(0o700)
    functions = []
    for script in ["install-server.sh", "install-agent.sh", "server-web-update.sh"]:
        source = (root / "scripts" / script).read_text()
        function = source.split("verify_release_attestation() {", 1)[1].split("\n}\n", 1)[0]
        functions.append(function)
        code = "verify_release_attestation() {" + function + "\n}\nverify_release_attestation candidate v1.2.3 && printf activated"
        env = dict(os.environ, PATH=str(work) + ":" + os.environ["PATH"], GH_ARGS=str(work / "args"), NODELITE_VERIFY_ATTESTATIONS="1")
        for result in ["0", "1"]:
            run = subprocess.run(["sh", "-c", code], env=dict(env, GH_RESULT=result), capture_output=True, text=True)
            assert (run.returncode == 0) == (result == "0"), (script, run.stderr)
            assert ("activated" in run.stdout) == (result == "0")
            args = (work / "args").read_text().splitlines()
            for expected in ["XiNian-dada/NodeLite", "XiNian-dada/NodeLite/.github/workflows/release.yml", "refs/tags/v1.2.3", "https://token.actions.githubusercontent.com", "--deny-self-hosted-runners"]:
                assert expected in args, (script, args)
        no_tag = subprocess.run(["sh", "-c", code.replace("candidate v1.2.3", "candidate ''")], env=dict(env, GH_RESULT="0"), capture_output=True)
        assert no_tag.returncode != 0
    assert len(set(functions)) == 1, "standalone verification policy drifted"
workflow = (root / ".github/workflows/release.yml").read_text()
assert workflow.index("- name: Attest release asset provenance") < workflow.index("- name: Create or update GitHub release")
assert "subject-path: publish/*" in workflow
assert 'if [ "$SOURCE_REF" != "refs/tags/$RELEASE_TAG" ]' in workflow
assert "id-token: write" in workflow and "attestations: write" in workflow
assert "continue-on-error" not in workflow[workflow.index("- name: Attest release asset provenance"):]
print("release attestation policy and failure closure passed")
