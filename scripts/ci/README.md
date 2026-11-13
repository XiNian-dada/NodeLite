# NodeLite Runner tools

The image is built by the coordinator on the mainland compiler, inside its existing DinD daemon. It does not change other repositories' Runner labels or open the host Docker socket to jobs.

Build context:

- `Dockerfile` from this directory.
- `debian-systemd.qcow2`, downloaded from Debian's official `https://cloud.debian.org/images/cloud/trixie/latest/debian-13-generic-amd64.qcow2`. The SHA-512 is pinned in the Dockerfile; a newer upstream image is rejected until reviewed.
- The existing `forgejo-ci-local:20261113-mirrors-final` base, whose download routes and Node.js 22 are managed by the Runner owner.

The resulting image is tagged `nodelite-ci:20261113-v1` only in the Runner's Docker daemon. No registry publication or credential is required. Rebuild with a new tag for tool changes and update the workflows together. Rust comes from a digest-pinned official image; Cargo jobs use two compilation workers within the Runner's existing resource limit.

`test-systemd-vm.py` starts the pinned Debian guest using QEMU TCG. Only a read-only payload ISO, a cloud-init ISO and a task-owned disk overlay are attached. No host directories, credentials, listeners, Docker socket or privileged container mode are passed into the guest. The guest's root can exercise network namespaces and systemd sandboxing without changing the host. The fixture uses the existing compiler proxy for Debian package installation. It must print `NODELITE_SYSTEMD_RESULT=0`; a boot, package, test or timeout failure fails CI and retains the serial log as an artifact.

macOS workflows are disabled as requested. Their former configuration remains in `.github/legacy-workflows`.
