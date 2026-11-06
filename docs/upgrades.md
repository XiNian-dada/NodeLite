---
title: 升级
nav_order: 7
---

# 升级

**先升级 Server，再升级所有 Agent。** 旧版 Agent 可以继续用有效凭据上报，但不支持凭据续期的落盘确认，请在 Token 过期前完成升级。凭据续期与恢复的细节见 [Agent 凭证续期与安全升级](agent-credential-upgrades.md)。

## 从 3.0.x 试用 3.1.0 RC

`v3.1.0-rc.1` 是需要主动选择的预发布版本；普通 `latest` 命令仍使用最新正式版。
请从目标 RC 下载安装器，并明确传入同一个 tag。不要用旧版或 `latest` 安装器试装 RC。

在 Server 主机执行：

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/download/v3.1.0-rc.1/install-server.sh | \
  sudo sh -s -- --mode upgrade --version v3.1.0-rc.1
```

Server 就绪后，在每台 Agent 主机执行：

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/download/v3.1.0-rc.1/install-agent.sh | \
  sudo sh -s -- --mode upgrade --version v3.1.0-rc.1
```

这两条命令使用现有安装：保留 Server 配置、节点注册表、数据库以及 Agent 的有效凭据，
不需要重新添加节点或生成安装令牌。自定义 Agent 路径仍需传入原来的目录参数；
自定义 Server 配置路径通过 `sudo env NODELITE_SERVER_CONFIG_PATH=/绝对路径/server.toml sh -s -- ...` 指定。
下载、校验或凭据预检失败时保留旧程序；启动失败时按各自的升级恢复流程回滚。
执行前应备份重要数据并为 Server 的完整备份预留空间。

也可按下一节通过 Server 生成 Agent 命令。自 3.1 起，官方构建生成的安装和升级命令
默认固定到该 Server 的发布 tag，包括 RC；开发构建才回退到 `latest`。
配置了 `[install]` 自定义镜像的 Server 继续使用镜像和固定摘要，请维护者先准备相应产物。
`--version` 优先于 `NODELITE_SERVER_VERSION` / `NODELITE_AGENT_VERSION`；环境变量仍然兼容。

本次手动升级不会自动启用网页 Agent 更新。已经迁移到签名更新器的节点继续遵循本地
签名策略，安装器的 `--version` 和下载源不能覆盖它；需等待对应签名版本发布。
尚未部署正式签名更新源的用户，先使用上述手动升级流程。
回退到 3.0.x 时不能只替换旧二进制，须恢复与旧版兼容的完整数据备份。

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

默认是上面的手动升级。Linux/systemd 节点可以改用签名更新器 [nodelite-updater](https://github.com/XiNian-dada/NodeLite/blob/main/nodelite-updater/README.md)。签名源可以直接使用 [GitHub Releases + Pages 托管](github-agent-updates.md)，普通用户无需自建服务器。首次开启需要项目维护者配置正式签名密钥并发布同版本迁移包。

从独立可信的发布审批渠道取得适合节点架构的 `enable-web-updates-<target>.sh` 和摘要记录，
核对版本、root、URL 和完整归档摘要后执行。该脚本先校验外层 installer，再传入固定的
`NODELITE_AGENT_VERSION`、`--signed-helper-sha256` 和 `--enrollment-sha256`。
如果只启用本地签名升级，把脚本中的 `--enable-web-updates` 改为 `--enable-signed-updates`。
不要从 `latest` 下载后直接执行首次迁移，也不要把同一下载站的 SHA256 清单当作独立信任。
维护者组包步骤见 [发布工具说明](https://github.com/XiNian-dada/NodeLite/blob/main/tools/nodelite_release/README.md)。

之后照常执行 `--mode upgrade`，已迁移的节点会走本机签名策略。

- 签名模式不接受面板下发的命令或下载地址，不会无人值守地自动拉取新版本。
- 仅支持默认路径、cgroup v2、未开启流量限速的非容器 Linux，上报间隔 1–10 秒；不满足条件时，预检会在停止旧 Agent 前拒绝。
- 更新器暂停或进入人工恢复状态时不会阻止 Agent 开机；时钟向前跳变导致下载被拒绝时，可用 `nodelite-updater reset-time-floor --confirm-clock-correct` 恢复。
- 生产签名源尚未配置，此功能要等迁移包随版本发布后才能使用。

## 可选：从面板升级 Agent

新节点按上面的经过独立审批的安装脚本启用。已经完成签名迁移、仅需允许网页触发的节点，
可以在本机执行已安装且经过校验的恢复入口，完成一次本地签名升级并启用网页触发：

```bash
sudo python3 -I /usr/local/lib/nodelite-operator/recover.py --upgrade --enable-web-updates
```

开启后可以在设置页或节点详情中发起升级。面板只发送触发信号，不能指定脚本、下载地址或安装路径；验签、切换和回滚仍由受限的更新器完成。操作需要二次确认，同一节点两次请求至少间隔 5 分钟。
