# Disposable Agent updater experiment

This is **not a production updater or installer**. It does not modify NodeLite's
workspace, Agent, Server, wire protocol, official installers, release workflow or UI.
The Python coordinator is an experiment, not production Rust implementation.

The permission/unit/transaction contract is in
[`docs/agent-update-isolation-prototype.md`](../../docs/agent-update-isolation-prototype.md).

## Safety boundary

- The compiler host only runs an unprivileged QEMU process and downloads/extracts
  tools into `~/NodeLite/output/agent-update-prototype/`. No host packages are installed.
- QEMU uses KVM, 2 vCPUs, 1 GiB RAM, an 8 GiB sparse overlay, no shared host directory,
  and restricted user networking. Only SSH is forwarded to host `127.0.0.1:22239`.
- No production server URL, SSH key, credential, database or Agent binary enters the guest.
- A disposable SSH key stays under the private output directory; only its public key
  enters cloud-init. Signing keys and random fixture config are generated inside the VM.
- Guest SSH explicitly disables agent forwarding and configured SSH tunnels.
- `setup_guest.py` refuses to run without the guest marker and QEMU/KVM detection.
- Every guest user, service and installed path uses the `nlup-lab` namespace.
- `vm.py stop` and `powercut` validate the QEMU PID's command line before SIGKILL;
  they never reboot or stop the compiler host.

## Reproduce on the configured compiler

First verify that `~/NodeLite` belongs to this project. Sync this directory only;
do not upload keys, `.env`, databases, `output/`, or unrelated local files.

The validated compiler has Ubuntu 22.04, a readable/writable `/dev/kvm`, LXD snap
runtime libraries, and Python 3.10. QEMU's snap build lacks user networking, so this
experiment extracts Ubuntu's `qemu-system-x86`, `qemu-system-common`,
`qemu-system-data`, `libslirp0` and `genisoimage` packages using `apt-get download`
and `dpkg-deb -x`, without installing them. The extracted packages reside in
`output/agent-update-prototype/tools/unpacked/`. `tools/lib/libnl-route-3.so.200`
is copied from the installed core20 snap. `vm.py` makes runtime paths explicit.
This host-specific runner is not portable deployment tooling.

Download the official Jammy cloud image and SHA256SUMS from
<https://cloud-images.ubuntu.com/jammy/current/> into the output directory.
`vm.py start` verifies its SHA256 before creating an overlay. This HTTPS/checksum
bootstrap is not presented as independent publisher-signature verification.

From the compiler's `~/NodeLite`:

```sh
cargo build --manifest-path experiments/agent-update-isolation/fixture/Cargo.toml --release --locked
cargo clippy --manifest-path experiments/agent-update-isolation/fixture/Cargo.toml --all-targets --locked -- -D warnings
python3 -m unittest discover -s experiments/agent-update-isolation -p test_io.py -v
python3 experiments/agent-update-isolation/vm.py start
python3 experiments/agent-update-isolation/vm.py push "$PWD/experiments/agent-update-isolation" /home/lab/
python3 experiments/agent-update-isolation/vm.py exec sudo python3 /home/lab/agent-update-isolation/setup_guest.py
python3 experiments/agent-update-isolation/vm.py exec sudo python3 /home/lab/agent-update-isolation/test_guest.py
python3 experiments/agent-update-isolation/powercuts.py
python3 experiments/agent-update-isolation/evidence.py
python3 experiments/agent-update-isolation/vm.py stop
```

Do not rerun setup concurrently with a test or pending transaction. Setup seeds a
new fixture baseline/config; it is lab provisioning, not update-time behavior.
The corrupt-journal boot test deliberately leaves the guest in manual recovery.
For reuse, run `test_guest.py reset-only` as root **inside the marked guest**;
this restores only fixture state and does not overwrite `results.json`.

## Evidence and limits

- Guest `/home/lab/results.json`: scenario names and terminal states, no credentials.
- Host `output/agent-update-prototype/powercuts.json`: checkpoint/recovery evidence.
- Serial output, disk overlay and temporary keys remain private on the compiler.
- ELF fixtures simulate healthy, crash, hang and preflight-rejection behaviors;
  they emit a heartbeat with PID, executable digest and a config hash, never config bytes.
- The coordinator checks systemd invocation/PID plus fixture heartbeat, not a remote panel ACK.
- The nine offline I/O checks cover hostile file types, retained writable source FDs,
  explicit object modes under umask, JSON ambiguity and injected fsync failures.
- The lab timer polls once per second for deterministic tests. It is not a measured
  production resource budget or final event/deadline implementation.
- No full TUF, key rotation, trusted-clock recovery, real network download path,
  actual NodeLite token handoff or protocol compatibility has been implemented here.
- `PREPARED` models the current Agent's quiescence via the fixture heartbeat. This
  is not proof of concurrent real token writes/ACKs or host-wide intrusion recovery.
- Power cuts are SIGKILL of QEMU at selected durable checkpoints, not arbitrary
  storage-controller failure or exhaustive crashes inside every syscall.

The update UID remains trusted. Compromising it can affect Agent code/config
indirectly, although no custom updater code runs as root. Productionization requires
an independent security review and integration tests beyond this experiment.
