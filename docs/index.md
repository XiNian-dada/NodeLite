---
title: 首页
nav_order: 1
permalink: /
---

# NodeLite

NodeLite 是一个用 Rust 编写的服务器监控面板，采用 Server-Agent 架构：每台被监控的机器运行一个 Agent，通过 WebSocket 把指标推送给 Server；Server 存储历史数据、评估告警，并提供内嵌的 Web 面板。

![NodeLite 面板](https://raw.githubusercontent.com/XiNian-dada/NodeLite/main/images/zh_cn/dark_home_page.png)

## 从哪里开始

| 我想…… | 看这里 |
| --- | --- |
| 先跑起来 | [快速开始](getting-started.md) |
| 放到域名和 HTTPS 后面 | [反向代理](reverse-proxy.md) |
| 了解有哪些配置项 | [配置](configuration.md) |
| 配置邮件或 Webhook 告警 | [告警与巡检](alerts.md) |
| 按流量套餐自动限速 | [流量限速](traffic-control.md) |
| 升级 Server 和 Agent | [升级](upgrades.md) |
| 排查节点不上线等问题 | [运维与排障](operations.md) |
| 看资源占用与性能数据 | [性能基准](benchmarks.md) |
| 参与开发 | [开发](development.md) |

## 组成

- **nodelite-server**：接收 Agent 连接，保存历史数据（SQLite），评估告警，提供 Web 面板、HTTP API 和 Prometheus `/metrics`。
- **nodelite-agent**：在被监控的 Linux / macOS 机器上采集 CPU、内存、磁盘、网络等指标，主动连接 Server，不需要开放入站端口。
- **nodelite-updater**（可选）：Linux/systemd 上的签名 Agent 更新器，默认关闭。

安全模型和认证细节见仓库中的 [SECURITY.md](https://github.com/XiNian-dada/NodeLite/blob/main/SECURITY.md)，架构说明见 [DESIGN.md](https://github.com/XiNian-dada/NodeLite/blob/main/DESIGN.md) 和[数据流示意图](architecture.html)。
