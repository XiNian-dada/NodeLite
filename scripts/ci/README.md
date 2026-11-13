# NodeLite Runner tools

Build `Dockerfile` in the compiler Runner's existing DinD daemon, using its existing `forgejo-ci-local:20261113-mirrors-final` base. That image supplies Node.js 22 and the owner's verified download routes. Rust 1.93.1 comes from the digest-pinned official image. The task installs pnpm, compiler tools and systemd once into `nodelite-ci:20261113-v2`; it does not publish the image or change other repositories' Runner labels.

`test-systemd-container.sh` runs only in a disposable Docker job. The repository-only `nodelite-systemd` Runner adds `SYS_ADMIN` (namespaces/cgroups/service sandboxing) and `NET_ADMIN` (traffic-control tests), with a private cgroup namespace. It starts systemd in a nested PID/mount namespace with its own `/run`. No host directory or Docker socket is mounted, and no privileged mode is used. The existing outer DinD isolation and 2 CPU / 3 GB / 512 PID job limits remain in place. Tests must write a zero exit status; their logs are preserved as artifacts. Ordinary jobs do not receive these extra capabilities.

macOS workflows are disabled at the owner's request. Their former configuration remains in `.github/legacy-workflows`. No QEMU or virtual disk is required by the active workflows.

The dedicated Runner is configured under `/home/compiler/NodeLite/ci-runner` on the compiler. It registers only with this repository, has capacity 1, and uses the same isolated DinD daemon. Its config and token are mode 0600. Runner-level options enforce the two capabilities because workflow container options are overridden by the main Runner configuration.
