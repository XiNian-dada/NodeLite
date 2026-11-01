---
title: 配置
nav_order: 4
---

# 配置

Server 的配置文件默认位于 `/opt/nodelite/config/server.toml`，带完整注释的模板见 [`config/server.example.toml`](https://github.com/XiNian-dada/NodeLite/blob/main/config/server.example.toml)；Agent 的模板见 [`config/agent.example.toml`](https://github.com/XiNian-dada/NodeLite/blob/main/config/agent.example.toml)。修改后重启服务生效。

`server.toml` 包含面板密码等凭据，建议权限设为 `600`；组或其他用户可读时，Server 启动会给出警告。

## 常用配置项

| 配置段 | 字段 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `[server]` | `listen` | `"127.0.0.1:8080"` | 监听地址。生产环境建议只监听本机，由反向代理对外提供服务。 |
| `[server]` | `public_base_url` | — | 对外访问地址，只能是来源（协议、主机、端口），不能带路径。用于生成 WebSocket 地址和安装命令。 |
| `[server]` | `insecure_allow_http` | `false` | 是否允许以明文 HTTP 对外提供面板，只用于隔离的测试环境。 |
| `[server]` | `trusted_proxies` | `[]` | 可信反向代理的网段，只有来自这些地址的 `X-Forwarded-For` 才会被采信。不接受 `0.0.0.0/0`。 |
| `[server]` | `stale_after_secs` | `20` | 多少秒没有收到心跳判定节点离线。 |
| `[server]` | `token_verify_max_parallelism` | `4` | 同时进行的 Argon2id 节点 Token 校验数，每个约占用 19 MiB 临时内存。 |
| `[auth]` | `username` / `password` | 安装时生成 | 面板登录凭据。 |
| `[auth]` | `enable_2fa` | `false` | 是否启用二次验证（TOTP，可再绑定 Passkey）。 |
| `[audit]` | `enabled` / `retention_days` / `max_rows` | `true` / `90` / `500000` | 安全审计日志的开关、保留天数和最大行数。 |
| `[metrics]` | `allow_basic_scrape_with_2fa` | `false` | 开启 2FA 后是否允许 Prometheus 只用 Basic 认证抓取 `/metrics`。 |
| `[geoip]` | `enabled` | `false` | 是否解析节点的地理位置，见下文。 |
| `[filters]` | `ignored_filesystems` | `["tmpfs", "devtmpfs", "overlay"]` | 面板中隐藏的文件系统类型。 |

文件路径（`node_registry_path`、`history_db_path` 等）如果写成相对路径，会按进程的工作目录解析，而不是配置文件所在目录。CLI 与 systemd 服务的工作目录不同时会读写不同的文件，生产环境请使用绝对路径；Server 启动时会对相对路径给出提示。

## 认证与二次验证

面板使用用户名和密码（HTTP Basic）登录。启用 `enable_2fa` 后，在面板的账户页绑定 TOTP；之后还可以添加 Passkey，用 Touch ID、Face ID 或设备锁屏完成二次验证，TOTP 保留作为恢复方式。

- Passkey 要求 `public_base_url` 使用 `https://`，浏览器需支持 WebAuthn。
- 已绑定的 Passkey 公开凭据保存在 `server.toml` 同目录的 `passkeys.json`（权限 `0600`），不要手工编辑。
- 修改密码、关闭 2FA、删除节点、调整节点流量限额等操作需要短时间内再次确认（密码、TOTP 或 Passkey）。
- 忘记密码或丢失 2FA 设备时的恢复方法见[运维与排障](operations.md#重置密码或关闭-2fa)。

## GeoIP

GeoIP 用于在地图上标出节点位置，默认关闭。

```toml
[geoip]
enabled = true
provider = "dbip"   # 或 "custom"（读取本地 database_path），或 "ipwhois"
```

- `dbip` / `custom` 使用本地 MMDB 数据库，不向外部发送数据。
- `ipwhois` 会把 Agent 的公网 IP 以及已登录访问者的 IP 发送给第三方服务 ipwho.is 查询，注重隐私或离线部署时不要使用。
- 兼容旧配置：只写了 `provider`、没有写 `enabled` 时视为开启。

## 安装源镜像

默认从 GitHub Releases 下载 Agent。需要使用自建镜像时配置 `[install]`，并同时提供两种架构的 SHA256，签发的安装和升级命令会自动带上镜像地址和固定校验值：

```toml
[install]
agent_release_base_url = "https://mirror.example.com/nodelite/v3.0.4"
agent_release_sha256_x86_64 = "<64 位十六进制>"
agent_release_sha256_aarch64 = "<64 位十六进制>"
```

远程 `http://` 镜像需要同时开启 `insecure_allow_http`。固定的校验值只覆盖 Linux musl 产物，macOS 安装会读取镜像中的 `SHA256SUMS.txt`。
