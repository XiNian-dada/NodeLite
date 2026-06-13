![NodeLite Banner](images/zh_cn/banner.png)

[**中文**](README.md) | [**English**](README.en.md)

[![CI](https://github.com/XiNian-dada/NodeLite/actions/workflows/ci.yml/badge.svg)](https://github.com/XiNian-dada/NodeLite/actions/workflows/ci.yml)
[![Coverage](https://github.com/XiNian-dada/NodeLite/actions/workflows/coverage.yml/badge.svg)](https://github.com/XiNian-dada/NodeLite/actions/workflows/coverage.yml)

# NodeLite

NodeLite 是一个用 Rust 编写的轻量级服务器监控面板，采用 Server-Agent 架构。它适合想要快速部署、低资源占用、以查看为主的服务器监控场景。

完整部署文档、进阶配置和架构图请看 GitHub Pages：
[https://xinian-dada.github.io/NodeLite/](https://xinian-dada.github.io/NodeLite/)

> **版本建议**：生产环境请使用 [GitHub Releases](https://github.com/XiNian-dada/NodeLite/releases) 中的最新正式版本，不建议使用 `-alpha`、`-beta`、`-rc` 预发布版本。

## 快速跳转

- [5 分钟安装](#5-分钟安装)
- [升级与日常运维](#升级与日常运维)
- [常见排障](#常见排障)
- [平台与部署拓扑](#平台与部署拓扑)
- [配置与安全边界](#配置与安全边界)
- [当前能力](#当前能力)
- [开发者入口](#开发者入口)
- [性能测试](#性能测试)
- [发布](#发布)
- [完整 HTML 文档](https://xinian-dada.github.io/NodeLite/)

## 5 分钟安装

推荐先跑通一台节点，再补 HTTPS、Prometheus、2FA 和其它生产配置。

1. 安装服务端：

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/latest/download/install-server.sh | sudo sh
```

2. 在服务端签发一条 Agent 安装命令：

```bash
/usr/local/bin/nodelite-server \
  --config /opt/nodelite/config/server.toml \
  install-agent \
  --node-id hk-01 \
  --node-label "Hong Kong 01"
```

3. 把上一步打印出的命令粘贴到目标子机执行。

完成后：

- 面板通过 `https://你的域名/` 访问
- Agent 通过 `wss://你的域名/ws` 接入
- 历史数据默认保留 14 天

更完整的部署步骤见 [快速部署文档](https://xinian-dada.github.io/NodeLite/#deploy)。

## 升级与日常运维

服务端升级：

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/latest/download/install-server.sh | \
  sudo NODELITE_SERVER_MODE=upgrade sh
```

Agent 升级命令可在服务端生成：

```bash
/usr/local/bin/nodelite-server \
  --config /opt/nodelite/config/server.toml \
  upgrade-agent
```

常用状态检查：

```bash
sudo systemctl status nodelite-server.service
sudo journalctl -u nodelite-server.service -f

sudo systemctl status nodelite-agent.service
sudo journalctl -u nodelite-agent.service -f
```

升级建议由管理员手动触发：先确认 release notes 和协议兼容，再升级 Server 或 Agent。

## 常见排障

- **面板能打开但没有节点**：先看 Agent 日志，重点检查 `wss://.../ws`、证书、反向代理 WebSocket 头和 node token。
- **子机提示 `invalid install token`**：一次性 install token 默认 15 分钟有效，重新执行 `install-agent` 即可。
- **服务端频繁出现 TLS 警告**：生产环境应放在 Nginx 或 Caddy 后面，用 HTTPS / WSS 对外访问。
- **Agent 被 `/ws` 限流挡住**：检查 `[ws]` 配额和 `server.trusted_proxies`，远端反代或 WAF 出口网段需要正确配置。
- **密码或 2FA 问题**：可通过服务端 `server.toml` 重置密码或关闭 2FA，然后重启服务。

更多问题见 [排障 FAQ](https://xinian-dada.github.io/NodeLite/#faq)。

## 平台与部署拓扑

- `nodelite-server`：推荐部署在 Linux + systemd。官方发布产物提供 `x86_64-unknown-linux-musl` 与 `aarch64-unknown-linux-musl`。
- `nodelite-agent`：支持 Linux 与 macOS。macOS 一键安装和 launchd 集成仍属实验性支持，建议先在测试机验证。
- 反向代理：生产环境推荐 Nginx 或 Caddy 终结 HTTPS / WSS。

推荐拓扑：

```text
Agent -> wss://monitor.example.com/ws -> Nginx/Caddy -> 127.0.0.1:nodelite-server
Browser -> https://monitor.example.com/ -> Nginx/Caddy -> 127.0.0.1:nodelite-server
```

## 配置与安全边界

- Web 面板和只读 API 使用 Basic Auth，可选 TOTP 2FA。
- Agent 使用逐节点 token 接入，token 存放在服务端注册表中。
- 敏感配置优先通过服务端文件、CLI 和受保护设置入口修改。
- `/metrics` 与面板共用只读认证，适合接入 Prometheus。
- 历史图用于展示基础趋势，不是每条 `metrics` 上报的完整归档。

Prometheus 快速验证：

```bash
curl -u viewer:secret https://monitor.example.com/metrics
```

## 当前能力

- 一键安装 Server / Agent
- 节点签发、token 轮换、Agent 手动升级命令生成
- 只读首页、节点详情页和 JSON API
- SQLite 短期历史、快照恢复、审计日志
- 可选 TOTP 2FA
- Prometheus `/metrics`
- 告警配置和每日巡检摘要配置
- Agent 指数退避重连

详细页面和架构图见 [HTML 文档](https://xinian-dada.github.io/NodeLite/#architecture)。

## 开发者入口

本地检查：

```bash
cargo check
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

交叉编译 Linux 静态二进制：

```bash
cargo build --release --target x86_64-unknown-linux-musl \
  -p nodelite-server \
  -p nodelite-agent

cargo build --release --target aarch64-unknown-linux-musl \
  -p nodelite-server \
  -p nodelite-agent
```

协议解析 fuzz smoke：

```bash
cargo test --manifest-path fuzz/Cargo.toml
cargo run --manifest-path fuzz/Cargo.toml --bin protocol_messages -- 10000
```

覆盖率：

```bash
cargo tarpaulin --config tarpaulin.toml
```

## 性能测试

性能基线建议用 release 构建在目标机器上重新跑，README 不再维护长表格，避免数据随版本漂移后增加阅读负担。

常用 loopback 压测：

```bash
cargo test -p nodelite-server --release load_test_scaling_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_api_surface_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_reconnect_storm_scores -- --ignored --nocapture
```

更大规模回归压测：

```bash
cargo test -p nodelite-server --release load_test_large_fleet_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_dashboard_fanout_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_history_pressure_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_payload_size_scores -- --ignored --nocapture
```

首页 DOM 渲染压力：

```bash
node scripts/benchmark-index-dom.mjs --nodes 500
node scripts/benchmark-index-dom.mjs --nodes 1000
```

## 发布

仓库使用 tag 驱动 GitHub Release。推送语义化版本 tag 后，CI 会构建 Linux Server / Agent、macOS Agent，上传安装脚本和 `SHA256SUMS.txt`，并创建 Release。

发布产物中的 Agent 会把对应 tag 版本号上报到面板，便于确认线上节点版本。
