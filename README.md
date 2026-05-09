# XiMonitor

XiMonitor 是一个用 Rust 编写的轻量级服务器监控面板，包含：

- `ximonitor-server`
  中心服务，提供 WebSocket 接入、只读页面、只读 JSON API、SQLite 短期历史和快照恢复。
- `ximonitor-agent`
  Linux agent，采集 CPU、负载、内存、磁盘、网络总流量、实时速率和 WebSocket RTT。
- `ximonitor-proto`
  服务端与 agent 共用的配置、协议和数据模型。

## 当前能力

- 服务端只读页面：
  - `/`
  - `/nodes/{node_id}`
- 服务端只读 API：
  - `/api/overview`
  - `/api/nodes`
  - `/api/nodes/{node_id}`
  - `/api/nodes/{node_id}/history`
- agent 接入协议：
  - `hello`
  - `metrics`
  - `ping`
  - `pong`
  - `server_notice`
- 72 小时 SQLite 历史保留
- 快照落盘与进程重启后恢复最近状态
- agent 指数退避自动重连

## 本地构建

```bash
cargo check
```

## 交叉编译 Linux x86_64 / aarch64

仓库内已经包含 musl 目标的 `lld` 链接配置，可以直接构建静态 Linux 二进制：

```bash
cargo build --release --target x86_64-unknown-linux-musl \
  -p ximonitor-server \
  -p ximonitor-agent

cargo build --release --target aarch64-unknown-linux-musl \
  -p ximonitor-server \
  -p ximonitor-agent
```

产物位置：

```bash
target/x86_64-unknown-linux-musl/release/ximonitor-server
target/x86_64-unknown-linux-musl/release/ximonitor-agent
target/aarch64-unknown-linux-musl/release/ximonitor-server
target/aarch64-unknown-linux-musl/release/ximonitor-agent
```

## 推荐部署拓扑

生产环境建议这样放：

1. `ximonitor-server` 监听在 `127.0.0.1:8080`
2. Nginx 或 Caddy 对外暴露 `443`
3. 面板和 API 走 HTTPS
4. Agent 通过 `wss://你的域名/ws` 接入

这样可以把 TLS、访问日志、限流和基础访问控制都放到反代层。

## 服务端部署

推荐直接用 GitHub Release 里的交互式安装器。它会清屏、询问安装目录、监听端口、对外域名或 IP、只读面板账号密码，然后自动：

- 按当前架构下载最新的 `ximonitor-server`
- 拉取 `SHA256SUMS.txt` 并校验二进制
- 生成 `server.toml` 和 `server.json`
- 注册并启动 `ximonitor-server.service`

一条命令安装：

```bash
curl -fsSL https://github.com/XiNian-dada/XiMonitor/releases/latest/download/install-server.sh | sudo sh
```

脚本默认会：

- 把程序数据放到你输入的安装目录下，默认建议 `/opt/ximonitor`
- 监听在 `127.0.0.1:<随机端口>`
- 要求你输入对外访问的域名或 IP，并据此生成 `public_base_url`
- 生成一组只读面板 Basic Auth 账号

安装完成后会直接打印：

- 服务端二进制路径
- 配置文件路径
- 节点注册表路径
- 面板只读用户名和密码
- 下一条该执行的 `install-agent` 签发命令

如果你更想手工部署，也可以：

1. 复制 [config/server.example.toml](/Users/bernard/Code/XiMonitor/config/server.example.toml) 和 [config/server.json.example](/Users/bernard/Code/XiMonitor/config/server.json.example)
2. 把服务端二进制安装到 `/usr/local/bin/ximonitor-server`
3. 手工创建 systemd unit
4. 启动 `ximonitor-server.service`

最少要确认的配置项是：

```toml
[server]
listen = "127.0.0.1:28080"
public_base_url = "https://monitor.example.com"
node_registry_path = "/opt/ximonitor/config/server.json"
history_db_path = "/opt/ximonitor/data/history.sqlite3"
snapshot_path = "/opt/ximonitor/data/snapshot.json"

[auth]
username = "viewer"
password = "change-this-password"

[ws]
max_total_connections = 1024
max_connections_per_ip = 32
auth_fail_window_secs = 300
auth_fail_max_attempts = 12
auth_block_secs = 900
```

查看服务端状态：

```bash
sudo systemctl status ximonitor-server.service
sudo journalctl -u ximonitor-server.service -f
```

## Nginx 反代示例

如果你用 Nginx，可以参考：

```nginx
server {
    listen 80;
    server_name monitor.example.com;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl http2;
    server_name monitor.example.com;

    ssl_certificate     /path/to/fullchain.pem;
    ssl_certificate_key /path/to/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location /ws {
        proxy_pass http://127.0.0.1:8080/ws;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 120s;
        proxy_send_timeout 120s;
    }

    location /install/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

## 节点签发

推荐直接在服务端执行 `install-agent` 子命令。它不会启动监听，只会修改 `server.json`、生成一次性 install token，然后打印一条可直接粘贴到子机上的安装命令。

```bash
/usr/local/bin/ximonitor-server \
  --config /opt/ximonitor/config/server.toml \
  install-agent \
  --node-id hk-01 \
  --node-label "Hong Kong 01" \
  --tag apac \
  --tag edge
```

这个命令会：

- 在 `server.json` 里创建或复用 `hk-01`
- 为该节点生成独立 token
- 生成一个 15 分钟有效的一次性 install token
- 直接打印一条完整的子机安装命令
- 让运行中的服务端在下一次注册表轮询时自动接纳新 token，无需重启进程

注意：

- `/`、`/nodes/*`、`/api/*` 默认受 HTTP Basic Auth 保护
- 安装脚本本身是公开静态文件；真正的节点配置通过一次性 install token 从 `/install/bootstrap` 拉取
- `install-agent` 只会把一次性 install token 内联到命令里，长期 node token 仍然只通过 bootstrap 响应体下发

如果你想看更详细的输出，包括 `agent.toml` 片段和到期时间，也可以继续使用 `issue-node`：

```bash
/usr/local/bin/ximonitor-server \
  --config /opt/ximonitor/config/server.toml \
  issue-node \
  --node-id hk-01 \
  --node-label "Hong Kong 01"
```

如果你需要轮换某个节点 token，可以追加 `--rotate-token`。

## 一键安装

服务端 `install-agent` 打印出来的命令大概长这样：

```bash
curl -fsSL https://monitor.example.com/install/install-agent.sh | \
  XIMONITOR_AGENT_INSTALL_TOKEN='one-time-token' sh -s -- \
  --bootstrap-url https://monitor.example.com/install/bootstrap \
  --base-url https://github.com/XiNian-dada/XiMonitor/releases/latest/download
```

说明：

- 脚本会检测架构并下载对应的 `ximonitor-agent-<target>` 二进制
- 脚本会自动下载 GitHub Release 的 `SHA256SUMS.txt`，并按当前架构校验 agent 二进制
- 一次性 install token 已经内联在命令里，所以正常情况下不需要再手工输入
- 长期 node token 只通过 bootstrap 响应体下发，不出现在 URL 或命令参数里
- 会创建 `ximonitor-agent` 专用系统用户，并以该用户运行 systemd service
- 会写入 `/etc/ximonitor/agent.toml`，并将目录/文件权限收紧到仅 root 与该服务用户可读
- 会生成 `ximonitor-agent.service`
- 会执行 `daemon-reload`、`enable` 和 `restart`

### 子机安装步骤

推荐按下面顺序操作：

1. 在服务端执行 `install-agent`
2. 复制它打印出的安装命令到目标 Linux 子机
3. 子机直接执行
4. 等脚本结束后检查服务状态

检查 Agent 服务：

```bash
sudo systemctl status ximonitor-agent.service
sudo journalctl -u ximonitor-agent.service -f
```

如果你想手工运行安装脚本，也可以：

```bash
sh scripts/install-agent.sh \
  --bootstrap-url https://monitor.example.com/install/bootstrap \
  --install-token <one-time-token> \
  --base-url https://github.com/XiNian-dada/XiMonitor/releases/latest/download
```

如果你已经有精确二进制地址，也可以继续使用自定义下载地址和校验文件：

```bash
sh scripts/install-agent.sh \
  --bootstrap-url https://monitor.example.com/install/bootstrap \
  --install-token <one-time-token> \
  --checksums-url https://your-host/releases/SHA256SUMS.txt \
  --binary-url https://your-host/releases/ximonitor-agent-x86_64-unknown-linux-musl
```

## 手工 Agent 启动

如果你暂时不想用安装脚本，也可以手工部署 agent。

1. 复制配置：

```bash
cp config/agent.example.toml config/agent.toml
```

2. 把 `node_id`、`node_label`、`server`、`token` 替换成服务端签发输出的内容。

3. 本机采样自检：

```bash
cargo run -p ximonitor-agent -- --config config/agent.toml --sample-once
```

4. 正常运行：

```bash
cargo run -p ximonitor-agent -- --config config/agent.toml
```

## 常见排障

- 面板能打开但没有节点，先看 Agent 日志里是不是 `wss://.../ws` 证书或反代问题。
- 如果服务端日志里频繁出现 TLS 警告，说明你还在用 `http://` 或 `ws://` 明文链路。
- 如果子机安装时提示 `invalid install token`，通常是一次性 token 过期了，重新执行一次 `install-agent` 或 `issue-node` 即可。
- 如果 Agent 被 `/ws` 限流挡住，先检查服务端 `[ws]` 配额是否太小，或者反代是否把所有请求都转成同一个源 IP。

## GitHub Release

仓库内置了一个 tag 驱动的发布工作流。当推送新的语义化版本 tag，例如 `1.0.0` 或 `v1.0.0` 时，GitHub Actions 会自动：

1. 交叉编译 Linux `x86_64-unknown-linux-musl`
2. 交叉编译 Linux `aarch64-unknown-linux-musl`
3. 生成 `ximonitor-server-x86_64-unknown-linux-musl`
4. 生成 `ximonitor-agent-x86_64-unknown-linux-musl`
5. 生成 `ximonitor-server-aarch64-unknown-linux-musl`
6. 生成 `ximonitor-agent-aarch64-unknown-linux-musl`
7. 上传 `install-server.sh` 和 `install-agent.sh`
8. 上传 `SHA256SUMS.txt`
9. 自动创建 GitHub Release

## 说明

- 网页端默认只读，不提供写配置入口。
- `/healthz` 和 `/ws` 不走只读面板鉴权；面板和 JSON API 走 HTTP Basic Auth；安装脚本和 bootstrap 接口使用独立安装流程。
- agent 只接受服务端 `server.json` 中已登记节点的逐节点 token。
- 首版 agent 只支持 Linux。
- 当前历史图保存基础趋势，不做长期归档。
- 生产环境建议放在 Nginx 或 Caddy 后面并启用 HTTPS。
