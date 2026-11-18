![NodeLite](images/en/banner.png)

[简体中文](README.md) | [English](README.en.md)

[![CI](https://akyuu.leeinx.com/XiNian-dada/NodeLite/actions/workflows/ci.yml/badge.svg)](https://akyuu.leeinx.com/XiNian-dada/NodeLite/actions?workflow=ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

# NodeLite

NodeLite is a server monitoring dashboard written in Rust. Each monitored machine runs an agent that pushes metrics to the server over WebSocket; the server stores history, evaluates alerts, and serves an embedded web dashboard.

📖 **Documentation**: [Repository docs](docs/index.md) · [Forgejo migration status](docs/forgejo.md)

The primary repository is [Forgejo](https://akyuu.leeinx.com/XiNian-dada/NodeLite). Actions builds a downloadable documentation site. Signed Agent updates remain paused until keys and state are recovered.

![Dashboard screenshot](images/en/dark_home_page.png)

## Features

- Live CPU, memory, disk, network, load and latency, with history stored in SQLite
- Server and agent are single static binaries; the web dashboard is embedded in the server. The agent runs on Linux and macOS
- Alerts via email (SMTP) and webhooks, with resolve notifications and a daily summary
- Monthly traffic quotas per node, with automatic bandwidth limiting through `tc` on Linux when the quota is exceeded
- Dashboard login with TOTP two-factor authentication and passkeys; agent tokens stored as Argon2id hashes; a separate security audit log
- Prometheus `/metrics` endpoint and a Grafana dashboard template
- Low resource use: about 10–15 MB for an idle server, about 2 MB per agent, about 514 MB for a server with 1,000 nodes ([benchmarks](docs/benchmarks.md))

## Quick start

Follow the [private Forgejo release instructions](docs/forgejo.md#从私有-release-安装或恢复升级) to install or upgrade the Server on a Linux host with systemd.

The server listens on `127.0.0.1:8080`. In production, put it behind a reverse proxy with HTTPS. With Caddy that is:

```caddy
monitor.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

With Nginx, forward WebSocket upgrades on `/ws` and keep the original `Host` header; see the [Nginx example](docs/reverse-proxy.md#nginx).

Issue an install command for each machine you want to monitor (it contains a one-time token valid for 15 minutes):

```bash
/usr/local/bin/nodelite-server --config /opt/nodelite/config/server.toml \
  install-agent --node-id hk-01 --node-label "Hong Kong 01"
```

Use the bootstrap URL and one-time token from the printed command with the [Agent installation steps](docs/forgejo.md#从私有-release-安装或恢复升级).

Use the same release to upgrade the Server first, then the Agents. Agents enrolled in the signed update channel require the [signed source](docs/forgejo.md#签名-agent-更新) to be restored.

## Configuration

The server reads `/opt/nodelite/config/server.toml`. The annotated templates [`config/server.example.toml`](config/server.example.toml) and [`config/agent.example.toml`](config/agent.example.toml) document every option. A few worth knowing:

- `[server] public_base_url` must be an origin (scheme, host, optional port) without a path.
- `[server] trusted_proxies` lists your reverse proxy networks; without it all agents behind a remote proxy share one IP and hit rate limits.
- `[geoip]` is off by default. The `ipwhois` provider sends agent and visitor IPs to the third-party service ipwho.is; use `dbip` or `custom` for local lookups.
- `[metrics] allow_basic_scrape_with_2fa = true` lets Prometheus scrape `/metrics` with Basic Auth when 2FA is enabled.

## Building from source

```bash
cargo build --release -p nodelite-server -p nodelite-agent
cargo test --workspace
```

The frontend lives in `nodelite-server/web` (Vue 3 + Vite) and is bundled into the server binary at build time. See the [development guide](docs/development.md).

## Contributing

Issues and pull requests are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md). Please report security issues privately as described in [SECURITY.md](SECURITY.md) rather than opening a public issue.

## License

[MIT](LICENSE)
