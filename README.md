![NodeLite Banner](images/zh_cn/banner.png)

[**简体中文**](README.md) | [**English**](README.en.md)

[![CI](https://github.com/XiNian-dada/NodeLite/actions/workflows/ci.yml/badge.svg)](https://github.com/XiNian-dada/NodeLite/actions/workflows/ci.yml)
[![Coverage](https://github.com/XiNian-dada/NodeLite/actions/workflows/coverage.yml/badge.svg)](https://github.com/XiNian-dada/NodeLite/actions/workflows/coverage.yml)
[![codecov](https://codecov.io/gh/XiNian-dada/NodeLite/branch/main/graph/badge.svg)](https://codecov.io/gh/XiNian-dada/NodeLite)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 2024](https://img.shields.io/badge/Rust-2024_Edition-orange.svg)](Cargo.toml)

# NodeLite

**NodeLite** 是一个用 Rust 编写的高性能、极轻量级服务器集群监控面板，采用标准的 Server-Agent 架构。

专为追求**极低系统资源占用**（服务端内存通常 < 15MB，Agent < 2MB）、**毫秒级实时数据流**（200+ 节点高吞吐并发下 p95 延迟 < 5ms）与**极简运维交付**（单静态二进制、内嵌 Vue 3 SPA）而设计。

📖 **官方完整部署文档与在线指南**：[https://xinian-dada.github.io/NodeLite/](https://xinian-dada.github.io/NodeLite/)

> [!TIP]
> **版本建议**：生产环境请使用 [GitHub Releases](https://github.com/XiNian-dada/NodeLite/releases) 中的最新正式版本（如 `v3.0.x`），测试环境可按需选用 `-rc` 或 `-beta` 预发布版本。

---

## 目录

- [✨ 核心特性](#-核心特性)
- [⚡ 5 分钟快速上手](#-5-分钟快速上手)
- [🏗️ 系统架构与数据流](#️-系统架构与数据流)
- [⚙️ 核心配置速查](#️-核心配置速查)
- [🚨 告警通知与 Linux 限速](#-告警通知与-linux-限速)
- [🔧 升级与日常运维](#-升级与日常运维)
- [❓ 常见问题与排障](#-常见问题与排障)
- [💻 开发者与源码构建](#-开发者与源码构建)

---

## ✨ 核心特性

* 🚀 **极低开销 & 单文件交付**：
  * 基于 Rust 构建，内存占用极其克制（Server 仅需十余 MB，Agent < 2MB），无任何 GC 停顿与隐形泄漏；
  * Vue 3 + TypeScript 前端 SPA 静态构建产物直接嵌入 Rust Server 二进制中，单文件交付，无需额外部署 Nginx 托管静态资源；
  * 提供 `musl` 静态二进制（`x86_64` / `aarch64`），零外部动态库依赖。
* ⚡ **毫秒级无锁集中广播**：
  * 首创**集中 Diff 广播引擎**，单一后台任务（1 秒去抖）统一计算节点增量差异，通过广播通道无锁扇出给全部浏览器会话；
  * 锁竞争与 Diff 复杂度由 $O(M \times N)$ 降至 $O(N)$，多端同时在看时 CPU 占用近乎平直。
* 🚦 **独家 Linux 套餐限速 (Traffic Control)**：
  * 深度集成 Linux 原生 `tc` 模块。当 VPS 月度流量达到预设配额时，Agent 可自动触发接口带宽限流，彻底避免公网流量超额扣费。
* 🛡️ **工业级安全防线**：
  * 节点 Token 采用 Argon2id 哈希，并配置有限并发池（防大批重连引发 OOM）；
  * 凭证与 Token 比较全面采用 `subtle::ConstantTimeEq` 防范时序侧信道攻击；
  * 控制台支持 Basic Auth + 可选 **TOTP 2FA** 动态口令；
  * 独立 SQLite 审计日志库（`audit.sqlite3`）记录全部鉴权与安全事件。
* 🚨 **完备的告警与每日巡检**：
  * 支持 CPU、内存、延迟、离线、月度流量用量等多维度规则评估；
  * 支持 **SMTP 邮件**（StartTLS）及 **Webhook**（Telegram / Discord / Slack / 自定义 Webhook）即时推送；
  * 每天 9:00 自动汇总结算过去 24 小时的健康度巡检摘要。
* 📈 **开放可观测性**：
  * 内置标准 Prometheus `/metrics` 抓取端点，并提供官方 Grafana 仪表盘模板；
  * 支持在线 API（ipwho.is）及本地 MaxMind / DB-IP 离线库（`.mmdb`）进行物理地理位置解析。

---

## ⚡ 5 分钟快速上手

### 步骤 1：一键安装服务端

在你的主控服务器（Linux，支持 systemd）上执行安装脚本：

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/latest/download/install-server.sh | sudo sh
```

脚本将自动检测 CPU 架构、校验 SHA256 哈希、生成默认配置与 systemd 单元文件，并自动启动 `nodelite-server`。

### 步骤 2：配置网络接入（反向代理）

`nodelite-server` 默认监听于 `127.0.0.1:8080`。生产环境推荐使用 **Nginx** 或 **Caddy** 终结 TLS 并代理 WebSocket。

<details open>
<summary><b>Nginx 配置示例（推荐）</b></summary>

```nginx
server {
    listen 443 ssl http2;
    server_name monitor.example.com; # 替换为你的域名

    ssl_certificate     /etc/letsencrypt/live/monitor.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/monitor.example.com/privkey.pem;

    # 1. 静态面板与 API
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # 2. Agent 与浏览器 WebSocket 连接 (关键配置)
    location /ws {
        proxy_pass http://127.0.0.1:8080/ws;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 120s;
        proxy_send_timeout 120s;
    }

    # 3. Agent 安装脚本分发
    location /install/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```
</details>

<details>
<summary><b>本地局域网测试（无需域名 / HTTP 直连）</b></summary>

若仅在测试环境验证，可修改 `/opt/nodelite/config/server.toml`：
```toml
[server]
listen = "0.0.0.0:8080"
public_base_url = "http://192.168.1.100:8080"
insecure_allow_http = true
```
重启服务后即可直接通过 `http://192.168.1.100:8080/` 访问。
</details>

### 步骤 3：签发并安装 Agent

在服务端执行签发命令，生成包含一次性 Token（15 分钟有效）的安装命令：

```bash
/usr/local/bin/nodelite-server \
  --config /opt/nodelite/config/server.toml \
  install-agent \
  --node-id hk-01 \
  --node-label "Hong Kong 01"
```

将服务端打印输出的 `curl ... | sh` 命令**直接复制到目标子机（Linux / macOS）上执行**。Agent 部署完毕后会自动与服务端完成握手上线，控制台将在 1~2 秒内自动刷出该节点。

---

## 🏗️ 系统架构与数据流

```text
[ 被控节点 Agent ]  ---(WSS: 指标快照/心跳)---> [ Nginx / Caddy 反向代理 ]
                                                       │
                                            (127.0.0.1:8080)
                                                       ▼
                                         [ nodelite-server 接入网关 ]
                                           ├── 准入控制 & Argon2id Token 鉴权
                                           ├── SharedState 集中 Diff 广播 (1s 去抖)
                                           ├── 异步 Batch Writer 写入 history.sqlite3
                                           └── 告警引擎 (SMTP / Webhook 派送)
                                                       │
                                   ┌───────────────────┴───────────────────┐
                                   ▼                                       ▼
                       [ 管理员浏览器 (Vue 3 SPA) ]              [ Prometheus /metrics ]
                       (接收增量 Diff 零锁渲染)                 (Grafana 官方看板拉取)
```

> 详细模块设计与锁优化细节可查阅 [`DESIGN.md`](DESIGN.md)。

---

## ⚙️ 核心配置速查

服务端配置文件位于 `/opt/nodelite/config/server.toml`（完整带注释模板见 [`config/server.example.toml`](config/server.example.toml)）：

| 配置段 | 关键参数 | 默认值 / 推荐值 | 作用说明 |
| :--- | :--- | :--- | :--- |
| `[server]` | `listen` | `"127.0.0.1:8080"` | 服务端监听地址，生产推荐监听本地回环 |
| `[server]` | `public_base_url` | `"https://monitor.example.com"` | 对外访问基准 URL，用于推导 WSS 与安装脚本路径 |
| `[server]` | `stale_after_secs` | `20` | 超过多少秒未收到心跳判定为节点离线 |
| `[server]` | `token_verify_max_parallelism` | `4` | Argon2id 验证并发槽位数（防重连尖刺 OOM，每任务约 19MB） |
| `[auth]` | `username` / `password` | 自动生成强密码 | 面板登录凭据（必须包含大小写字母、数字和特殊字符） |
| `[auth]` | `enable_2fa` | `false` | 是否启用 TOTP 二次验证 |
| `[audit]` | `enabled` / `retention_days`| `true` / `90` | 独立安全审计日志，默认留存 90 天 |
| `[geoip]` | `provider` | `"ipwhois"` | 物理位置查询：`"ipwhois"`（在线）/ `"dbip"`（本地 MMDB） |

---

## 🚨 告警通知与 Linux 限速

### 1. 告警配置（SMTP 邮件与 Webhook）

编辑 `/opt/nodelite/config/server.toml` 中的 `[alerts]` 配置段：

```toml
[alerts]
enabled = true

# 邮件渠道 (支持 StartTLS)
[alerts.smtp]
enabled = true
host = "smtp.example.com"
port = 587
username = "alert@example.com"
password = "your-smtp-password"
sender = "alert@example.com"
recipients = ["admin@example.com"]
send_resolved = true

# Webhook 渠道 (Telegram / Discord / 自定义机器人)
[alerts.webhook]
enabled = true
url = "https://api.telegram.org/bot<TOKEN>/sendMessage?chat_id=<CHAT_ID>"
send_resolved = true

# 每日巡检报告
[alerts.inspection]
enabled = true
local_time = "09:00" # 每天上午 9 点发送
delivery = ["smtp", "webhook"]
```

### 2. Linux Agent 套餐超额限速 (tc)

在安装或升级 Agent 时加上 `--enable-traffic-control` 参数即可开启内核级带宽限流功能：

```bash
# 在子机安装命令末尾附加参数
curl -fsSL https://monitor.example.com/install/install-agent.sh | \
  NODELITE_AGENT_INSTALL_TOKEN='...' sh -s -- \
  --enable-traffic-control
```

* **权限最小化**：服务以非 root 用户 `nodelite-agent` 运行，仅保留 `CAP_NET_ADMIN` 和 `AF_NETLINK`，保留完整的 systemd 沙箱隔离；
* **联动控制**：在 Web 仪表盘设置节点月流量上限与结算日后，一旦超出流量配额，Agent 将自动调用 Linux `tc` 限制网卡带宽，告警系统将同步发出通知。

---

## 🔧 升级与日常运维

### 常用状态检查

```bash
# 查看服务端状态与近期日志
sudo systemctl status nodelite-server.service
sudo journalctl -u nodelite-server.service -f

# 查看 Agent 状态与通信日志
sudo systemctl status nodelite-agent.service
sudo journalctl -u nodelite-agent.service -f
```

### 服务端与 Agent 平滑升级

* **服务端升级**（自动备份配置与 SQLite 数据，版本验证通过后生效）：
  ```bash
  curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/latest/download/install-server.sh | \
    sudo NODELITE_SERVER_MODE=upgrade sh
  ```
* **Agent 批量升级命令生成**：
  ```bash
  /usr/local/bin/nodelite-server --config /opt/nodelite/config/server.toml upgrade-agent
  ```

### 忘记管理员密码 / 重置 2FA

直接修改 `/opt/nodelite/config/server.toml` 中的 `[auth]` 配置段并重启服务：
```toml
[auth]
password = "NewStrongPassword@2026"
enable_2fa = false # 若丢失 2FA 设备，置为 false 即可关闭
```
```bash
sudo systemctl restart nodelite-server.service
```

---

## ❓ 常见问题与排障

- **面板能打开但节点列表为空？**
  - 查看 Agent 日志：`sudo journalctl -u nodelite-agent -n 50 --no-pager`；
  - 检查反向代理是否正确转发了 WebSocket 协议升级头（`Upgrade` 与 `Connection "upgrade"`）。
- **子机安装提示 `invalid install token`？**
  - 签发的一次性安装 Token 有效期为 15 分钟。超时后重新在服务端执行 `install-agent` 即可。
- **Agent 被 `/ws` 限流拦截？**
  - 如果使用了远端反代或 CDN/WAF，请在 `server.toml` 的 `trusted_proxies` 中添加代理 IP 网段，否则多台 Agent 会被识别为同一个代理 IP 并触发频率限制。
- **Prometheus 抓取配置？**
  - `/metrics` 与控制台共享只读认证，在 `prometheus.yml` 中添加 `basic_auth` 即可，详见 [`ops/prometheus/prometheus.yml`](ops/prometheus/prometheus.yml)。

---

## 💻 开发者与源码构建

### 本地编译与测试

```bash
# 静态代码检查与全量测试
cargo check
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

### 编译 Linux 静态 Musl 二进制

```bash
cargo build --release --target x86_64-unknown-linux-musl -p nodelite-server -p nodelite-agent
cargo build --release --target aarch64-unknown-linux-musl -p nodelite-server -p nodelite-agent
```

### 协议模糊测试 (Fuzzing)

```bash
cargo test --manifest-path fuzz/Cargo.toml
cargo run --manifest-path fuzz/Cargo.toml --bin protocol_messages -- 10000
```

---

## 开源协议

本项目采用 [MIT 许可证](LICENSE)。欢迎提交 Issue 与 Pull Request！
