# The existing Runner image supplies its verified download routes and Node.js 22.
FROM rust:1.93.1-bookworm@sha256:7c4ae649a84014c467d79319bbf17ce2632ae8b8be123ac2fb2ea5be46823f31 AS rust
FROM forgejo-ci-local:20261113-mirrors-final
COPY --from=rust /usr/local/rustup /usr/local/rustup
COPY --from=rust /usr/local/cargo /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends \
    clang lld cmake pkg-config python3-venv python3-yaml \
    shellcheck jq iproute2 tini file systemd systemd-sysv dbus \
    libssl-dev ca-certificates curl git make gcc g++ && \
    npm install --global pnpm@10.11.0
