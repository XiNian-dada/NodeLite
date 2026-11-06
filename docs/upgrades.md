---
title: 升级
nav_order: 7
---

# 升级

**先升级 Server，再升级所有 Agent。** 旧版 Agent 可以继续用有效凭据上报，但不支持凭据续期的落盘确认，请在 Token 过期前完成升级。凭据续期与恢复的细节见 [Agent 凭证续期与安全升级](agent-credential-upgrades.md)。

官方 [GitHub 签名更新源](https://xinian-dada.github.io/NodeLite/updates/)已经上线，当前提供
`v3.1.0-rc.1` 的 Linux x86_64 / ARM64 `testing` 通道。普通 `latest` 仍选择最新正式版，
`stable` 签名通道将在正式发布时另行启用。

| 当前情况 | 操作 |
| --- | --- |
| Server 仍为 3.0.x | 先执行下面固定 RC 版本的 Server 升级命令 |
| 符合签名更新条件的 Linux Agent，已有版本至少 3.0.4 | 执行一次[固定版本启用命令](github-agent-updates.md#一次升级并启用网页更新)，升级到 RC 并启用网页更新 |
| Agent 已手动升级到 RC，但未启用网页更新 | 执行同一份启用命令，保留现有配置和有效凭据 |
| Agent 已启用网页更新 | 在节点详情 → 设置 → Agent 升级中发起更新 |
| macOS、容器、自定义路径或不满足签名条件的节点 | 继续使用下面的手动 Agent 升级命令 |

## 从 3.0.x 试用 3.1.0 RC

`v3.1.0-rc.1` 是需要主动选择的预发布版本；普通 `latest` 命令仍使用最新正式版。
请从目标 RC 下载安装器，并明确传入同一个 tag。不要用旧版或 `latest` 安装器试装 RC。

在 Server 主机执行：

```bash
curl -fsSL https://github.com/XiNian-dada/NodeLite/releases/download/v3.1.0-rc.1/install-server.sh | \
  sudo sh -s -- --mode upgrade --version v3.1.0-rc.1
```

Server 就绪后，默认路径、满足[签名更新条件](#启用签名和网页更新)且已有版本至少 3.0.4 的
Linux Agent，可直接执行[固定版本启用命令](github-agent-updates.md#一次升级并启用网页更新)，
一次完成 RC 升级和网页更新启用。

其他节点，或希望继续手动升级的节点，在 Agent 主机执行：

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

手动升级不会自动启用网页 Agent 更新；需要网页更新时，升级后再执行一次上述启用命令。
3.0.0–3.0.3 的 Agent 也应先手动升级，再在满足条件时启用。
已经迁移到签名更新器的节点继续遵循本地签名策略，安装器的 `--version` 和下载源不能覆盖它；
当前 `testing` 源已授权 `3.1.0-rc.1`，后续版本需先完成签名发布。
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

<a id="可选签名自动更新"></a>

## 启用签名和网页更新

官方签名源和两种 Linux 架构的首次启用包现已发布，普通用户无需配置密钥或自建服务器。
在已有 Agent 主机执行[带固定摘要的启用命令](github-agent-updates.md#一次升级并启用网页更新)，
脚本自动选择架构，校验外层 installer 和迁移包，安装本机签名更新器并允许网页触发。
已经手动升级到 RC 的节点也需要这一步；更换 Agent 二进制本身不会开启网页更新。

首次执行前从可信发布审批记录核对脚本及 root 的固定摘要。该命令已固定到 `v3.1.0-rc.1`
和当前 GitHub 源，不要改用 `latest`，也不要只在旧安装命令后追加 `--enable-web-updates`。
如果只启用本地签名升级，把脚本中的 `--enable-web-updates` 改为 `--enable-signed-updates`。
维护者组包步骤见 [发布工具说明](https://github.com/XiNian-dada/NodeLite/blob/main/tools/nodelite_release/README.md)；
更新器的实现和恢复接口见 [nodelite-updater](https://github.com/XiNian-dada/NodeLite/blob/main/nodelite-updater/README.md)。

之后可从网页主动发起更新，也可照常执行 `--mode upgrade`，已迁移的节点会走本机签名策略。
更新源每六小时自动续签；Agent 仍需本地或网页主动发起更新，不会无人值守地自动安装新版本。

- 签名模式不接受面板下发的命令或下载地址，不会无人值守地自动拉取新版本。
- 仅支持默认路径、cgroup v2、未开启流量限速的非容器 Linux，上报间隔 1–10 秒；不满足条件时，预检会在停止旧 Agent 前拒绝。
- 更新器暂停或进入人工恢复状态时不会阻止 Agent 开机；时钟向前跳变导致下载被拒绝时，可用 `nodelite-updater reset-time-floor --confirm-clock-correct` 恢复。
- 主机需能访问 GitHub Pages，下载有大小和时间限制。超时后先恢复网络再重试同一命令；保留现有迁移和信任状态。

## 可选：从面板升级 Agent

已有普通安装的节点，先执行上面的固定版本启用命令。已经完成签名迁移、仅需允许网页触发的节点，
可以在本机执行已安装且经过校验的恢复入口，完成一次本地签名升级并启用网页触发：

```bash
sudo python3 -I /usr/local/lib/nodelite-operator/recover.py --upgrade --enable-web-updates
```

开启后进入**节点详情 → 设置 → Agent 升级**，查看当前版本、本地升级渠道和升级状态，再点击
“升级 Agent”。当前 RC 源已安装相同版本时，会返回“已是本地授权的最新版本”。

当前面板没有独立的“网页更新已启用 / 未启用”标识；“等待操作”只表示没有正在执行的更新。
未启用时会提示先在节点开启网页签名升级，本地策略未允许时会显示相应原因。
按钮暂时不可用也可能是节点离线、请求冷却或正在升级，不能仅凭按钮颜色判断启用情况。

面板只发送触发信号，不能指定脚本、下载地址或安装路径；验签、切换和回滚仍由受限的更新器完成。
操作需要二次确认，同一节点两次请求至少间隔 5 分钟。
