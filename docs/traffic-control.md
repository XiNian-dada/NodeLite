---
title: 流量限速
nav_order: 6
---

# 流量限速

在面板里给节点设置月流量上限和限速值后，用量达到上限时，Linux Agent 会用 `tc` 把出口网卡限速到设定值，同时可以配合 `traffic_usage_percent` 规则发出告警。

## 开启

安装或升级 Agent 时加上 `--enable-traffic-control`：

```bash
curl -fsSL https://monitor.example.com/install/install-agent.sh | \
  NODELITE_AGENT_INSTALL_TOKEN='...' sh -s -- \
  --enable-traffic-control
```

Agent 仍以非 root 用户 `nodelite-agent` 运行，只额外获得 `CAP_NET_ADMIN` 和 `AF_NETLINK`，其余 systemd 沙箱限制保持不变。限速规则只作用于 NodeLite 自己添加的过滤器；默认路由切换到另一块网卡时，规则会迁移到新网卡。

## 在面板中设置

在设置页的节点列表或节点详情的设置标签中，填写月流量上限、计费方向（双向 / 仅入站 / 仅出站）和超额后的限速值。

- 这些设置需要 Server 配置了 `[auth]`，保存时需要二次确认。
- 用量按 **UTC 自然月**累计，每月 1 日（UTC）重新开始。

## 计费细节

- 首次上报建立基线，用量从 0 开始；Agent 重启导致计数器归零不会产生负用量。
- 主机重启、网卡计数器重置或回绕时有专门的处理，避免重复计费。
- 流量账本存储不可用时，已限速的节点保持当前状态，`/readyz` 会显示 `degraded`。

完整规则和已知限制见[套餐账本与节点注销](traffic-accounting.md)。
