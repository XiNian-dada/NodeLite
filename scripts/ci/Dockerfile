# The existing Runner image supplies its verified download routes and Node.js 22.
FROM rust:1.93.1-bookworm@sha256:7c4ae649a84014c467d79319bbf17ce2632ae8b8be123ac2fb2ea5be46823f31 AS rust
FROM forgejo-ci-local:20261113-mirrors-final
COPY --from=rust /usr/local/rustup /usr/local/rustup
COPY --from=rust /usr/local/cargo /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends \
    clang lld llvm-dev cmake pkg-config python3-venv python3-yaml \
    shellcheck jq iproute2 tini file qemu-system-x86 qemu-utils cloud-image-utils \
    libssl-dev ca-certificates curl git make gcc g++ && \
    npm install --global pnpm@10.11.0

COPY debian-systemd.qcow2 /opt/nodelite-ci/debian-systemd.qcow2
RUN echo "ec8c175e39d59e711798c3a2f22ba11b9aee9bca1887c7e505a800aeffd5d7616099bd0e9fad869efba96100fe09d2c8b49d6d7001d501f0056c556bdc25b605  /opt/nodelite-ci/debian-systemd.qcow2" | sha512sum -c -
