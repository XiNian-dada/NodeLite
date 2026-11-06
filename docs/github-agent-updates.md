---
title: GitHub 托管 Agent 更新
nav_order: 8
---

# GitHub 托管 Agent 更新

**不需要另外租服务器或配置域名。** GitHub Releases 保存发布产物，现有
[GitHub Pages 文档站](https://xinian-dada.github.io/NodeLite/)同时提供静态签名更新文件。
初次启用需要维护者配置正式签名密钥；仓库中的空配置不代表更新源已经上线。

## 普通用户如何使用

先把 Server 升级到 3.1。维护者发布正式签名源及首次启用包后，从独立可信的发布审批记录
取得对应版本、架构的 `enable-web-updates-<target>.sh` 及摘要，在已有 Agent 主机执行一次。
脚本保留现有配置和有效凭据，安装更新器并允许网页触发。之后可以在网页设置或节点详情
点击更新，不需要每次登录节点，也不需要部署更新服务器。

可用脚本将列在 [更新源入口](https://xinian-dada.github.io/NodeLite/updates/)；
源未激活时这个地址可能返回 404，请继续使用[手动升级](upgrades.md)。
RC 使用 `testing` 通道；正式版使用 `stable`。本机固定通道，不会因为面板操作误装 RC。
macOS、容器和其他不满足签名更新器预检条件的节点继续手动升级。

## 托管方式

| 内容 | 位置 | 校验方式 |
| --- | --- | --- |
| 原始发布二进制 | GitHub Releases 的版本 tag | 离线 targets 签名绑定长度和 SHA256 |
| 根和发布审批 | `main` 的 `updates/approved/<channel>/<target>/metadata/` | root、targets 各至少两把独立密钥 |
| 在线元数据及历史 | `codex/agent-update-state` 分支 | 验签、版本递增、不可变历史比较 |
| Agent 下载入口 | Pages 的 `updates/<channel>/<target>/{metadata,targets}/` | 本机 TUF 验签和摘要校验 |
| 首次启用交接包 | 同一 Release 的额外 `nodelite-web-enrollment.tar.gz` | 配置中独立审批的整包摘要 |

GitHub Release 下载会重定向。发布工作流负责下载并核对签名摘要，然后把 hash 命名的
Agent 文件放进 Pages。Agent 保持禁止重定向和限制下载路径的规则。
状态分支只保存公开元数据；二进制从 Release 重建，避免将二进制历史反复提交进 Git。
原始 `SHA256SUMS.txt` 和 provenance 不修改；交接包的清单仅用于自己的目录。

Pages 每次完整部署包含原有 Jekyll 文档与更新文件。工作流共用一个并发组，不取消正在
发布的任务。签名状态先正常提交、推送，再部署 Pages；推送竞争会失败，不会强推覆盖。
部署失败可重新运行工作流，已签名历史仍在状态分支。

## 维护者首次配置

### 1. 完成密钥保管和离线审批

按[发布工具说明](https://github.com/XiNian-dada/NodeLite/blob/main/tools/nodelite_release/README.md)
生成正式密钥、根元数据与各范围的 targets 审批。不要复用实验密钥。
建议 root 为 2-of-3，targets 为 2-of-2；保管人独立持有密钥，离线密钥不进入 Actions。
snapshot、timestamp 使用另外两把互不复用的在线密钥。

每个启用范围提交所有连续的 `N.root.json` 和经过审批的 `N.targets.json`，例如：

```text
updates/approved/testing/x86_64-unknown-linux-musl/metadata/1.root.json
updates/approved/testing/x86_64-unknown-linux-musl/metadata/1.targets.json
```

`updates/github.json` 示例（摘要必须替换为独立核对过的真实值）：

```json
{
  "schema": 1,
  "repositories": [
    {
      "channel": "testing",
      "target": "x86_64-unknown-linux-musl",
      "root_sha256": "独立确认的初始根文件SHA256"
    }
  ],
  "enrollments": []
}
```

`root_sha256` 是持久信任锚。根轮换时保留旧根、补齐双重签名的新根，不重置这个锚或
元数据版本。历史引用的 Release 文件必须保留且字节不变。已发布范围不能从配置中删除，
也不能通过重新初始化绕过回退保护。

### 2. 配置两个受限 GitHub Environments

创建 `nodelite-update-snapshot` 和 `nodelite-update-timestamp`，将部署分支限制为 `main`。
每个环境只配置自己的 `NODELITE_ONLINE_PRIVATE_KEY`（加密 PKCS8 PEM）和
`NODELITE_ONLINE_PASSWORD`。不要放到仓库级 secrets，不要上传 root 或 targets 私钥。

例如 snapshot 环境，私钥从文件输入，口令通过 `gh` 的交互提示输入：

```bash
gh secret set NODELITE_ONLINE_PRIVATE_KEY --repo XiNian-dada/NodeLite \
  --env nodelite-update-snapshot < /安全位置/snapshot.pem
gh secret set NODELITE_ONLINE_PASSWORD --repo XiNian-dada/NodeLite \
  --env nodelite-update-snapshot
```

timestamp 环境使用自己的密钥重复操作。无人值守续签时，这两个在线环境不能要求每六小时
人工批准；使用分支限制和工作流代码审查控制签名代码的变更。
环境隔离不等于独立保管：同一仓库管理员仍能修改工作流，真正的新版本授权始终保留在
离线 targets 阈值签名中。

仅对**在线角色**提供明确的自动化例外：GitHub 在签名步骤将环境级 secret 传入 Python，
进程立即从环境中移除，在内存中解密，不写私钥文件、不启动子进程、不输出密钥或口令。
依赖在该步骤之前安装。离线命令继续只接受交互口令或 `--password-fd`，不适用这个例外。

### 3. 建立持久状态分支

确认远端尚无 `codex/agent-update-state` 后，在**新的空目录**建立状态 checkout。
不要在日常工作区执行 orphan 切换：

```bash
git clone --no-checkout https://github.com/XiNian-dada/NodeLite.git nodelite-update-state
cd nodelite-update-state
git switch --orphan codex/agent-update-state
printf '%s\n' 'Public signed metadata. Never reset or force-push this branch.' > README.md
git add README.md
git commit -m 'chore: initialize public Agent update state'
git push origin HEAD
```

限制写入者，禁止强推和删除，但允许发布工作流的 `GITHUB_TOKEN` 正常推送。
分支缺失或状态丢失时先核对最新公开版本和历史；不要用空目录或旧备份自动恢复。

### 4. 发布首次启用包

使用与发布版本匹配的已审查 checkout，按 `stage-web-release` 流程准备交接目录，
把 `--release-url` 指向该版本的 Pages 目录。RC 的 x86_64 范围例如：

```text
--channel testing
--tag v3.1.0-rc.1
--metadata-url https://xinian-dada.github.io/NodeLite/updates/testing/x86_64-unknown-linux-musl/metadata/
--targets-url https://xinian-dada.github.io/NodeLite/updates/testing/x86_64-unknown-linux-musl/targets/
--release-url https://xinian-dada.github.io/NodeLite/updates/enrollment/v3.1.0-rc.1/assets
```

两种架构一起启用时，把两组仓库、根摘要和地址一起传给 `stage-web-release`。
原始 Release 文件先验证 provenance、完整清单和对应 tag 的安装器。交接目录只包含
公开文件；取得独立审批后，用新工具打包，作为额外资产上传：

```bash
python3 -m tools.nodelite_release.github pack-handoff \
  --source /已审批的交接目录 --out /发布目录/nodelite-web-enrollment.tar.gz
gh release upload v3.1.0-rc.1 /发布目录/nodelite-web-enrollment.tar.gz \
  --repo XiNian-dada/NodeLite
```

不使用 `--clobber`，不覆盖 RC 原有资产。在 `enrollments` 加入
`{"tag":"v3.1.0-rc.1","sha256":"独立审批的整包SHA256"}`，正常审查后合并到 `main`。
从独立可信渠道分发启用脚本和摘要；同一站点的 SHA 清单不能单独建立首次信任。

### 5. 激活并验证

Settings → Pages 的 Source 使用 **GitHub Actions**，保留原域名和路径。
准备齐全后，把仓库变量 `NODELITE_GITHUB_UPDATES` 设置为 `enabled`，手动运行
“Documentation and signed Agent updates”。仅首次启用新范围时勾选 `initialize`。
后续保持关闭；不能把初始化当作丢失状态的恢复办法。

检查工作流成功、原文档首页正常、`updates/status.json` 的版本/有效期、根指纹和启用脚本。
再让测试节点完成首次迁移与网页更新，确认后向用户宣布可用。

## 后续发布、续签和恢复

新版本仍由两位发布审批人签署 targets，递增 metadata version 和 release sequence，
保持 rollback floor 不下降。审批合并到 `main` 后，工作流获取对应 `v<version>` Release
的 `nodelite-agent-<target>`，校验后发布。仅创建 Release 不会自动获得更新授权。

每六小时分别更新 snapshot 和 timestamp。GitHub 定时调度可能延迟或因仓库长期不活跃
停用，请关注 Actions 失败通知和 `updates/status.json`。timestamp 为 12 小时，snapshot
为 7 天；targets 接近 90 天、root 接近一年时需提前离线续签。过期会拒绝新更新，不会
停掉健康运行的 Agent。在线角色过期后可以续签；离线角色过期需先提交新的离线审批。

状态保存成功而部署失败时可重新运行工作流。配置已包含更新范围，但启用变量被移除时，
流程会失败，避免用只有文档的站点覆盖更新源。签名记录或 Release 文件丢失、版本回退、
摘要不符、软链接和未知文件都会阻止发布。

保留历史以兼容 CDN 缓存与跨版本请求。更新资产有 900 MiB 预算，整站小于 1 GB；超限
会阻止发布，需要规划归档或迁移，不能自动删除旧信任历史。备份状态分支和各自的离线
密钥；不要回退分支或强推来修复更新失败。
