---
title: 运维与排障
nav_order: 8
---

# 运维与排障

## 查看状态与日志

```bash
sudo systemctl status nodelite-server
sudo journalctl -u nodelite-server -f

sudo systemctl status nodelite-agent
sudo journalctl -u nodelite-agent -f
```

`/readyz` 返回 Server 的就绪状态；各项诊断的含义见[就绪检查](readiness.md)。

## 重置密码或关闭 2FA

编辑 `server.toml` 的 `[auth]` 段后重启：

```toml
[auth]
password = "NewStrongPassword@2026"
enable_2fa = false   # 丢失 TOTP / Passkey 设备时关闭，之后可重新绑定
```

```bash
sudo systemctl restart nodelite-server
```

## Prometheus

`/metrics` 与面板使用同一套认证，抓取配置示例见 [`ops/prometheus/prometheus.yml`](https://github.com/XiNian-dada/NodeLite/blob/main/ops/prometheus/prometheus.yml)，Grafana 仪表盘模板在 [`ops/grafana/`](https://github.com/XiNian-dada/NodeLite/tree/main/ops/grafana)。

- 开启 2FA 后，Prometheus 无法完成二次验证，`/metrics` 默认返回 401。需要在 `[metrics]` 中设置 `allow_basic_scrape_with_2fa = true`，这只放宽 `/metrics`。
- 默认只导出概览和少量节点指标；`[metrics]` 中的 `export_node_resource_metrics`、`export_node_disk_metrics` 会按节点导出更多序列，节点多时序列数增长很快。

## 常见问题

**面板能打开，但没有节点。**
查看 Agent 日志：`sudo journalctl -u nodelite-agent -n 50 --no-pager`。确认反向代理转发了 WebSocket 升级头（`Upgrade` 与 `Connection "upgrade"`），见[反向代理](reverse-proxy.md)。

**安装时提示 `invalid install token`。**
一次性安装 Token 15 分钟内有效，过期后在 Server 上重新执行 `install-agent`。

**Agent 被 `/ws` 限流拒绝。**
经过远端反向代理、CDN 或 WAF 时，把代理网段加入 `trusted_proxies`，否则所有 Agent 会被识别为同一个 IP。

**面板实时数据不更新，日志里有 foreign origin。**
反向代理没有保留原始 `Host`，或者代理端口不是默认端口却用了 Nginx 的 `$host`。见[反向代理](reverse-proxy.md#浏览器来源校验)。

**内存占用看起来比宣称的大。**
`systemctl status` 里的内存（cgroup `MemoryCurrent`）包含 SQLite 产生的可回收文件缓存，不等于进程自身占用。测量方法见[内存测量](memory-measurement.md)。

**节点磁盘列表缺少 NFS / CIFS 等网络挂载。**
Agent 默认跳过网络文件系统，避免挂载无响应时阻塞采集。需要显示时，在 Agent 配置中改写 `[agent].ignored_filesystems`；单个挂载无响应时会沿用上次的容量。
