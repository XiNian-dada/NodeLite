# Disposable Agent updater experiment

This is **not a production updater or installer**. It does not modify NodeLite's
production Agent, Server, wire protocol, official installers, release workflow or UI.
The only workspace hook is a Linux `#[cfg(test)]`, explicitly ignored integration test.
The Python coordinator is an experiment, not production Rust implementation.

The permission/unit/transaction contract is in
[`docs/agent-update-isolation-prototype.md`](../../docs/agent-update-isolation-prototype.md).

## Safety boundary

- The compiler host only runs an unprivileged QEMU process and downloads/extracts
  tools into `~/NodeLite/output/agent-update-prototype/`. No host packages are installed.
- QEMU uses KVM, 2 vCPUs, 1 GiB RAM, an 8 GiB sparse overlay, no shared host directory,
  and restricted user networking. Only SSH is forwarded to host `127.0.0.1:22239`.
- No production server URL, SSH key, credential, database or deployed binary enters the guest.
  The default real-credential suite builds Agent ELF files from this checkout with
  `lab-old` / `lab-new` labels. The optional released-binary suite uses fixed official
  v3.0.3/v3.0.4 assets; neither suite uses a production installation or credentials.
- A disposable SSH key stays under the private output directory; only its public key
  enters cloud-init. Signing keys and random fixture config are generated inside the VM.
- Guest SSH explicitly disables agent forwarding and configured SSH tunnels.
- `setup_guest.py` refuses to run without the guest marker and QEMU/KVM detection.
- Every guest user, service and installed path uses the `nlup-lab` namespace.
- `vm.py stop` and `powercut` validate the QEMU PID's command line before SIGKILL;
  they never reboot or stop the compiler host.

## Reproduce on the configured compiler

First verify that `~/NodeLite` belongs to this project. Fixture-only runs need this
directory; real-credential runs also need the matching tracked Rust workspace,
including `nodelite-server/tests/integration/mod.rs`. Do not upload keys, `.env`,
databases, `output/`, or unrelated local files.

The validated compiler has Ubuntu 22.04, unified cgroup v2, a readable/writable `/dev/kvm`, LXD snap
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

## Real Agent credential suite

After provisioning the base experiment and starting the VM, run on the compiler:

```sh
python3 experiments/agent-update-isolation/run_real.py
python3 experiments/agent-update-isolation/vm.py stop
```

For actual release compatibility, run `releases.py` on the compiler, then
`run_real.py --official` while the provisioned VM is running. Downloads are fixed
to GitHub tags v3.0.3/v3.0.4, bounded by curl timeouts, and checked against each
release's SHA256SUMS.txt (including its directory-prefixed filenames). This is
HTTPS/checksum bootstrap, **not independent publisher-signature verification**.
The authenticated Agent's reported version and executable digest are checked.

The released-binary matrix has eight cases. v3.0.3 cannot acknowledge durable token
renewal, so online renewal must be rejected without changing its file or process.
Upgrade to v3.0.4 first; only then rotate credentials. The matrix also tests
rolling the program back to v3.0.3 after v3.0.4 saved a new token, retaining that
new configuration. Remaining cases cover permissions, stale disk credentials,
and three interrupted coordinator checkpoints. Its server remains the production
WS/registry test harness, not the full CLI server. Results are written separately
to `output/agent-update-prototype/official-real-evidence.json`.

The runner builds two release Agent binaries and a server test executable, then
checks that the exact ignored test exists and copies them into the guest. Prior
guest result output is cleared so a zero-test run cannot reuse old evidence.
The test refuses to provision anything outside the
explicitly marked root/KVM/cgroup-v2 guest. Do not run its ignored Rust test on the host.
The compiler needs an existing `web/dist/index.html` for backend-only compilation;
the suite does not build or test the UI.

Inside the guest, the real Agent runs under the same unprivileged `nlup-agent`
systemd service and content-addressed updater layout. The candidate's read-only
preflight uses existing `--check-credentials`, not an authenticated formal session.
`STOPPING` waits for systemd inactivity, MainPID zero and an empty cgroup before
the active object changes; `KillMode=control-group` also covers children. The
import bound is 32 MiB to accommodate ordinary release binaries.
The fixed root stop action must also finish and consume its request. Cold boot
clears old action markers before the late-start bridge paths activate; setup
verifies the unit graph before enabling units.

The server executable uses production WebSocket, registry, persistence and refresh
handlers via existing `TestServer`; it is not the full production CLI/background-task
deployment. A loopback-only relay can withhold a refresh response or durability ACK.
Random credentials remain inside the guest; control input uses stdin, never argv.

A **trusted test observer**, not production Agent IPC, converts independently
observed process digest/invocation and fresh, authenticated Server metrics into the
prototype heartbeat. It rejects metrics older than the observed process invocation
or with the wrong build label. This does not implement production health attestation,
nor make a compromised Server a trustworthy signal.

Cases cover real-process permissions, rejected stale disk credentials without
displacing the healthy RAM session, rotation/upgrade, rotation during Trial and rollback, a permission-denied
write, a killed process before delivery, killed after durable write but before ACK
delivery, withheld ACK during update, and updater death at three handoff checkpoints.
Each successful recovery checks an actual authenticated metrics report and accepts
the disk credential against a newly loaded registry. It does not inject a kill
inside every write/fsync syscall or simulate a full Server restart.

The real suite stops lab services on exit but leaves its fixed test drop-ins and
fake config on the private VM disk. To return to fixtures, run the guarded action:

```sh
python3 experiments/agent-update-isolation/vm.py exec sudo python3 /home/lab/agent-update-isolation/real_guest.py restore-fixture
```

This stops lab units, removes their two `real.conf` drop-ins and the lab's temporary
power-cut delay override, then reruns base setup. Never run fixture reset against
the real-suite drop-ins.

## Evidence and limits

- Guest `/home/lab/results.json`: scenario names and terminal states, no credentials.
- Host `output/agent-update-prototype/powercuts.json`: checkpoint/recovery evidence.
- Serial output, disk overlay and temporary keys remain private on the compiler.
- ELF fixtures simulate healthy, crash, hang and preflight-rejection behaviors;
  they emit a heartbeat with PID, executable digest and a config hash, never config bytes.
- The fixture coordinator checks systemd invocation/PID plus fixture heartbeat;
  the real suite supplies the trusted test observer described above.
- The nine offline I/O checks cover hostile file types, retained writable source FDs,
  explicit object modes under umask, JSON ambiguity and injected fsync failures.
- Eight offline coordinator regressions cover retained cgroup children, root stop
  completion, boot marker cleanup and health progress. A cached recent heartbeat
  is insufficient: both elapsed stability and
  advancing evidence must span the window; future/stale samples or a new invocation
  restart that window. The real observer uses collection time, not polling time.
- The lab timer polls once per second for deterministic tests. It is not a measured
  production resource budget or final event/deadline implementation.
- The installed coordinator still uses the lab packet format, not TUF. A separate
  reference-TUF HTTP experiment is described below; it is not wired to activation.
  Trusted-clock recovery and production health IPC remain unimplemented.
- The full stop barrier avoids concurrent old/new formal Agent writers. These
  selected credential windows are not proof of host-wide intrusion recovery.
- Power cuts are SIGKILL of QEMU at selected durable checkpoints, not arbitrary
  storage-controller failure or exhaustive crashes inside every syscall.

The update UID remains trusted. Compromising it can affect Agent code/config
indirectly, although no custom updater code runs as root. Productionization requires
an independent security review and integration tests beyond this experiment.

## Signed HTTP download experiment

`tuf_network.py` runs **unprivileged on compiler loopback**, in a private temporary
directory. It uses the [reference python-tuf client](https://theupdateframework.readthedocs.io/en/latest/api/tuf.ngclient.updater.html)
and metadata API, rather than claiming the coordinator's custom signature packet
is full TUF. Optional, test-only dependency versions are pinned in
`lab-requirements.txt`. No dependency is added to the Rust workspace or installers.
On this compiler without pip/ensurepip, an official `pip.pyz` downloaded from
`https://bootstrap.pypa.io/pip/pip.pyz` bootstraps a `--without-pip` venv under
`output/agent-update-prototype/python-env`; no host package is installed. Install
the requirements with `--only-binary=:all:` in that venv, then run:

```sh
output/agent-update-prototype/python-env/bin/python experiments/agent-update-isolation/tuf_network.py
```

Sixteen real HTTP cases use the official Agent assets as payloads: successful
download, hash corruption, truncation, oversized payload/metadata, insufficient
targets signatures, expiry, scope mismatch, timestamp replay, signed application
downgrade, same-release equivocation, mixed metadata, successful root rotation,
insufficient old/new root signatures, and revoked targets keys. Results and code
hashes go to `tuf-network-evidence.json`. Root requires 2-of-3 keys; targets requires
2-of-2. All test keys are generated in memory in **one test process**, not operated
by independent release custodians. The mirror is HTTP intentionally: authenticity
comes from the bootstrapped root, not from trusting the mirror's transport.

This validates reference-library behavior and a fixed-scope release floor, not a
production signing service, durable TUF cache, trusted-clock repair, or integrated
downloader. Notably, a truncated stream raises urllib3 `ProtocolError` in this
library combination; callers must fail closed on transport errors as well as TUF
verification errors. There is no activation on any rejected download.
