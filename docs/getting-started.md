---
title: 快速开始
nav_order: 2
---

# 快速开始

本页把 Server 装好、接入第一台 Agent。生产环境还需要配置[反向代理](reverse-proxy.md)。

## 准备

- 一台 Linux（systemd）主机运行 Server。生产环境建议有一个解析到它的域名，并通过反向代理提供 HTTPS。
- 被监控的机器（Linux 或 macOS）只需要能出站访问 Server 的 HTTPS 端口，不需要开放入站端口。
- 生产环境请使用 [GitHub Releases](https://github.com/XiNian-dada/NodeLite/releases) 中的正式版本；`-rc`、`-beta` 为预发布版本。

## 1. 安装 Server

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/latest/download/install-server.sh | sudo sh
```

安装器会识别 CPU 架构、校验下载文件的 SHA256、询问安装目录（默认 `/opt/nodelite`）、监听地址、对外访问地址和管理员密码，然后生成 systemd 服务并启动 `nodelite-server`。

Server 默认监听 `127.0.0.1:8080`，需要通过[反向代理](reverse-proxy.md)对外提供访问。

### 只在局域网里试用

如果只是在内网测试、不想配置域名和 HTTPS，可以让 Server 直接监听局域网地址。编辑 `/opt/nodelite/config/server.toml`：

```toml
[server]
listen = "0.0.0.0:8080"
public_base_url = "http://192.168.1.100:8080"
insecure_allow_http = true
```

重启服务后访问 `http://192.168.1.100:8080/`。明文 HTTP 只适合隔离的测试环境。

## 2. 接入 Agent

在 Server 上签发安装命令，命令里包含一个 15 分钟内有效的一次性 Token：

```bash
/usr/local/bin/nodelite-server \
  --config /opt/nodelite/config/server.toml \
  install-agent \
  --node-id hk-01 \
  --node-label "Hong Kong 01"
```

把输出的 `curl ... | sh` 命令复制到目标机器上执行。Agent 用一次性 Token 换取长期凭据，保存在权限为 `0600` 的配置文件中，之后自动连接 Server，面板会在几秒内显示这台机器。

如果安装源是远程 `http://` 地址（例如开启了 `insecure_allow_http` 的 Server），签发出的命令会自动带上 `--insecure-allow-http`；手工拼接命令时需要自己加上。

## 3. 检查

- `systemctl status nodelite-server` 显示 `active (running)`。
- 浏览器打开面板地址，用安装时设置的账号密码登录。
- Agent 安装完成后，节点出现在面板上并开始显示指标。
- 访问 `/metrics` 能拿到 Prometheus 格式的指标（需要登录认证）。

节点没有出现时，见[运维与排障](operations.md#常见问题)。
