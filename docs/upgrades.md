---
title: 升级
nav_order: 7
---

# 升级

**先升级 Server，再升级所有 Agent。** 旧版 Agent 可以继续用有效凭据上报，但不支持凭据续期的落盘确认，请在 Token 过期前完成升级。凭据续期与恢复的细节见 [Agent 凭证续期与安全升级](agent-credential-upgrades.md)。

## 升级 Server

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/latest/download/install-server.sh | \
  sudo NODELITE_SERVER_MODE=upgrade sh
```

安装器会校验新版本、备份配置和数据库，预检失败时不替换当前程序。失败恢复的细节见 [Server 升级与失败恢复](server-upgrades.md)。

## 升级 Agent

在 Server 上生成升级命令，再到各节点执行：

```bash
/usr/local/bin/nodelite-server --config /opt/nodelite/config/server.toml upgrade-agent
```

安装脚本会：

- 只从 HTTPS（或本机回环）地址下载，所有下载都有大小和超时限制；远程 `http://` 源需要显式加 `--insecure-allow-http`。
- 校验下载文件的 SHA256 和磁盘上已保存的凭据，失败时保留正在运行的旧版本。
- 替换前备份旧程序和服务文件；重启后约 10 秒内持续检查服务是否在运行，不在运行就恢复旧版本并重启，然后以失败退出。中途出错或被信号中断时同样会恢复。

## 可选：签名自动更新

默认是上面的手动升级。Linux/systemd 节点可以改用签名更新器 [nodelite-updater](https://github.com/XiNian-dada/NodeLite/blob/main/nodelite-updater/README.md)，只需开启一次：

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/latest/download/install-agent.sh | \
  sudo sh -s -- --mode upgrade --enable-signed-updates
```

之后照常执行 `--mode upgrade`，已迁移的节点会走本机签名策略。

- 签名模式不接受面板下发的命令或下载地址，不会无人值守地自动拉取新版本。
- 仅支持默认路径、cgroup v2、未开启流量限速的非容器 Linux，上报间隔 1–10 秒；不满足条件时，预检会在停止旧 Agent 前拒绝。
- 更新器暂停或进入人工恢复状态时不会阻止 Agent 开机；时钟向前跳变导致下载被拒绝时，可用 `nodelite-updater reset-time-floor --confirm-clock-correct` 恢复。
- 生产签名源尚未配置，此功能要等迁移包随版本发布后才能使用。

## 可选：从面板升级 Agent

在节点上用 `--enable-web-updates` 开启一次（包含签名迁移）：

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/latest/download/install-agent.sh | \
  sudo sh -s -- --mode upgrade --enable-web-updates
```

开启后可以在设置页或节点详情中发起升级。面板只发送触发信号，不能指定脚本、下载地址或安装路径；验签、切换和回滚仍由受限的更新器完成。操作需要二次确认，同一节点两次请求至少间隔 5 分钟。
