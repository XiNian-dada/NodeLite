![NodeLite Banner](images/en/banner.png)

[**中文**](README.md) | [**English**](README.en.md)

[![CI](https://github.com/XiNian-dada/NodeLite/actions/workflows/ci.yml/badge.svg)](https://github.com/XiNian-dada/NodeLite/actions/workflows/ci.yml)
[![Coverage](https://github.com/XiNian-dada/NodeLite/actions/workflows/coverage.yml/badge.svg)](https://github.com/XiNian-dada/NodeLite/actions/workflows/coverage.yml)

# NodeLite

NodeLite is a lightweight server monitoring dashboard written in Rust with a Server-Agent architecture. It is designed for quick deployment, low resource usage, and mostly read-only operations.

Full deployment docs, advanced configuration, and architecture diagrams are available on GitHub Pages:
[https://xinian-dada.github.io/NodeLite/](https://xinian-dada.github.io/NodeLite/)

> **Version recommendation**: use the latest stable version from [GitHub Releases](https://github.com/XiNian-dada/NodeLite/releases) in production. Avoid `-alpha`, `-beta`, and `-rc` pre-releases unless you are testing.

## Quick Jump

- [5-Minute Install](#5-minute-install)
- [Upgrades and Operations](#upgrades-and-operations)
- [Troubleshooting](#troubleshooting)
- [Platforms and Topology](#platforms-and-topology)
- [Configuration and Security Boundaries](#configuration-and-security-boundaries)
- [Current Capabilities](#current-capabilities)
- [Developer Entry Points](#developer-entry-points)
- [Performance Tests](#performance-tests)
- [Release](#release)
- [Full HTML Docs](https://xinian-dada.github.io/NodeLite/)

## 5-Minute Install

Start by getting one node online, then add HTTPS, Prometheus, 2FA, and other production settings.

1. Install the server:

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/latest/download/install-server.sh | sudo sh
```

2. Issue an Agent install command on the server:

```bash
/usr/local/bin/nodelite-server \
  --config /opt/nodelite/config/server.toml \
  install-agent \
  --node-id hk-01 \
  --node-label "Hong Kong 01"
```

3. Paste the printed command on the target machine.

After installation:

- Dashboard: `https://your-domain/`
- Agent WebSocket: `wss://your-domain/ws`
- History retention: 14 days by default

See the [quick deployment guide](https://xinian-dada.github.io/NodeLite/#deploy) for the full flow.

## Upgrades and Operations

Upgrade the server:

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/latest/download/install-server.sh | \
  sudo NODELITE_SERVER_MODE=upgrade sh
```

Generate an Agent upgrade command on the server:

```bash
/usr/local/bin/nodelite-server \
  --config /opt/nodelite/config/server.toml \
  upgrade-agent
```

Common status checks:

```bash
sudo systemctl status nodelite-server.service
sudo journalctl -u nodelite-server.service -f

sudo systemctl status nodelite-agent.service
sudo journalctl -u nodelite-agent.service -f
```

Upgrades should be triggered explicitly by an operator after checking release notes and protocol compatibility.

## Troubleshooting

- **Dashboard opens but no nodes appear**: check Agent logs first, especially `wss://.../ws`, certificates, reverse proxy WebSocket headers, and node token mismatches.
- **Target machine reports `invalid install token`**: the one-time install token is valid for 15 minutes by default; run `install-agent` again.
- **Server logs TLS warnings repeatedly**: put the server behind Nginx or Caddy and expose HTTPS / WSS in production.
- **Agent is blocked by `/ws` rate limits**: check `[ws]` quotas and `server.trusted_proxies`; remote reverse proxy or WAF egress ranges must be configured correctly.
- **Password or 2FA issues**: reset the password or disable 2FA in `server.toml`, then restart the server.

More details are in the [troubleshooting FAQ](https://xinian-dada.github.io/NodeLite/#faq).

## Platforms and Topology

- `nodelite-server`: recommended on Linux + systemd. Official builds provide `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`.
- `nodelite-agent`: supports Linux and macOS. macOS one-command install and launchd integration are still experimental; test before long-term use.
- Reverse proxy: Nginx or Caddy is recommended for HTTPS / WSS termination.

Recommended topology:

```text
Agent -> wss://monitor.example.com/ws -> Nginx/Caddy -> 127.0.0.1:nodelite-server
Browser -> https://monitor.example.com/ -> Nginx/Caddy -> 127.0.0.1:nodelite-server
```

## Configuration and Security Boundaries

- Web dashboard and read-only API use Basic Auth, with optional TOTP 2FA.
- Agents connect with per-node tokens stored in the server registry.
- Sensitive settings are primarily changed through server-side config files, CLI commands, and protected settings endpoints.
- `/metrics` shares read-only authentication with the dashboard and can be scraped by Prometheus.
- History charts show basic trends; they are not a full archive of every `metrics` report.
- GeoIP is enabled by default with the online `ipwhois` provider. The Server sends the resolved public connection IP of an Agent to the third-party endpoint `https://ipwho.is` to look up country/region and, when available, city and coordinates. Private, loopback, and documentation-only addresses are classified as LAN locally and are not sent to that endpoint.

To avoid sending public IPs to a third party, set `geoip.enabled = false`, or switch `geoip.provider` to `dbip` / `custom` and supply a local MMDB file. Keep `geoip.auto_update = false` as well when GeoIP operation must not make any network requests.

Quick Prometheus check:

```bash
curl -u "$NODELITE_READONLY_USERNAME:$NODELITE_READONLY_PASSWORD" https://monitor.example.com/metrics
```

## Current Capabilities

- One-command Server / Agent installation
- Node issuing, token rotation, and Agent upgrade command generation
- Read-only dashboard, node detail page, and JSON API
- SQLite short-term history, snapshot recovery, and audit log
- Optional TOTP 2FA
- Prometheus `/metrics`
- Alert configuration and daily inspection summary configuration
- Agent exponential backoff reconnects

Detailed pages and diagrams are in the [HTML docs](https://xinian-dada.github.io/NodeLite/#architecture).

## Developer Entry Points

Local checks:

```bash
cargo check
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

Cross-compile static Linux binaries:

```bash
cargo build --release --target x86_64-unknown-linux-musl \
  -p nodelite-server \
  -p nodelite-agent

cargo build --release --target aarch64-unknown-linux-musl \
  -p nodelite-server \
  -p nodelite-agent
```

Protocol parser fuzz smoke:

```bash
cargo test --manifest-path fuzz/Cargo.toml
cargo run --manifest-path fuzz/Cargo.toml --bin protocol_messages -- 10000
```

Coverage:

```bash
cargo tarpaulin --config tarpaulin.toml
```

## Performance Tests

Performance baselines should be rerun with release builds on the target machine. The README no longer keeps long benchmark tables because those numbers drift as the code evolves.

Memory baselines don't use a single usage number. Server and Agent reports must
separate process RSS/PSS, anonymous resident memory, and systemd cgroup
`MemoryCurrent`; the latter includes reclaimable filesystem page cache created
by SQLite and WAL I/O and isn't a Rust heap or leak metric. See the
[memory measurement methodology](docs/memory-measurement.md) for commands,
cold/warm scenarios, and the report template. Unscoped `Server <15MB` and
`Agent <2MB` statements are no longer release gates.

Common loopback benchmarks:

```bash
cargo test -p nodelite-server --release load_test_scaling_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_api_surface_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_reconnect_storm_scores -- --ignored --nocapture
```

Larger regression benchmarks:

```bash
cargo test -p nodelite-server --release load_test_large_fleet_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_dashboard_fanout_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_history_pressure_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_payload_size_scores -- --ignored --nocapture
```

Homepage DOM rendering pressure:

```bash
node scripts/benchmark-index-dom.mjs --nodes 500
node scripts/benchmark-index-dom.mjs --nodes 1000
```

## Release

Releases are tag-driven. Pushing a semantic version tag builds Linux Server / Agent binaries, macOS Agent binaries, install scripts, `SHA256SUMS.txt`, and creates a GitHub Release.

Official builds strip the leading `v` from the tag and inject the result as `NODELITE_BUILD_VERSION`. Release Agent binaries report that value as `agent_version` to the dashboard, and the Server settings API exposes the same value as `server_version`.

The workspace `Cargo.toml` version `0.1.0` is the fallback used when no build version is injected; it is not changed for each release. A local `cargo build` or `cargo run` without `NODELITE_BUILD_VERSION` therefore shows the Cargo fallback in those runtime fields, which does not identify the version of an official Release asset.
