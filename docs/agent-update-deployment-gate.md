# 签名更新的受控部署准入

本规程不授权或执行现网部署。代码验证记录见
[安全审查](agent-update-security-validation.md)。首批只考虑已验证的
x86_64 / Ubuntu 22.04 / systemd 249 / cgroup v2 / 无 capabilities 默认安装；不能据此
批准其他发行版、aarch64、容器、tc、自定义单元或批量迁移。

## 部署前必须由管理员落实

1. 真实独立的 root 2-of-3 保管人与 targets 2-of-2 审批人；snapshot/timestamp 使用不同
   受限密钥。隔离验证仅使用一次性密钥，不能转为生产密钥。各保管人独立备份和演练恢复，
   不把满足阈值的私钥集中到同一 CI 或面板。
2. 独立核对构建源码、正常 updater、12 个固定单元、完整 enrollment 包和 root 指纹。
   特权 updater 和单元不由 Agent 的 targets 签名替代审查；禁止 lab-faults 构建。
3. 独立的只读 HTTPS 命名空间；按渠道/架构拆仓库，HTTP 用户不能写，发布工作区、归档、
   私钥不在 web root。仓库/归档所有祖先由 root 或发布者保护。地址固定在本地 root 策略，
   不从 Server/Web 接收下载 URL 或 shell 命令。
4. timestamp 至少每六小时续签；snapshot/targets/root 在到期前按不同权限刷新。定时执行
   `tools.nodelite_release check`，非零退出接管理员告警。确认人实际接收一次演练告警，
   不能只安装定时任务却无人值守。当前代码提供命令，不代替生产调度/告警服务。
5. 有磁盘容量预算、公开历史保留和私有归档备份规程。`collect` 默认计划、显式 apply，
   保留至少三代及不短于实际 HTTP 缓存/读者寿命的宽限。它只移动旧代次，不删除数据，
   当前代次内的历史产物仍增长，必须监控字节数、inode 和 64 代次上限。
6. 节点时间同步正常，有控制台/SSH 故障恢复通道和维护窗口。旧安装至少 v3.0.4，专属
   用户、配置 0600/目录 0700，无附加特权或自定义 drop-in。不能为了通过检查放宽权限。

这些项目没有在本轮替管理员创建；缺少任何一项均不得全量上线。

## 一台 canary 的验收顺序

在维护窗口选择一台符合条件、允许失败时人工恢复的节点。先保留旧程序和 unit 的可信记录；
**不把旧 Token/配置作为升级回滚备份**。凭证恢复走独立的重签发规程，而非回滚旧 Token。

从 root 保护的已审查源码执行迁移检查（参数示例见[迁移说明](../tools/nodelite_migrate/README.md)），
确认 root 和完整包摘要均独立可信，再以相同参数追加 `--apply`。迁移成功后执行：

```sh
sudo python3 -m tools.nodelite_migrate --audit
sudo -u nodelite-updater -g nodelite-read /usr/local/bin/nodelite-updater status
sudo -u nodelite-updater -g nodelite-read /usr/local/bin/nodelite-updater download
```

确认 download 不重启当前 Agent；仅在存在已审查、已双签的后续版本时，由本地管理员执行：

```sh
sudo systemctl start nodelite-updater-apply.service
sudo python3 -m tools.nodelite_migrate --audit
```

确认真实 Server 鉴权上报持续推进、版本/摘要符合预期、Token 权限未变，随后安排一次正常
节点重启并再次 audit。canary 至少跨越一次在线 metadata 续签周期，确认续签不需要离线
targets 密钥、旧 Agent 不被不必要重启，之后再决定是否扩大到下一小批节点。

`--audit` 不读取配置/Token，不修改服务或凭证；status 可能维护自身锁和 journal 标记。
若 Agent/updater/root 已失陷，健康自报不是远程证明，不能用 ready 宣称主机未被入侵。

## 停止推广的条件

- 任何非 ready、签名/指纹不符、manual_recovery、非预期重启/重复会话、Token 持久化失败；
- 元数据到期/刷新失败、根链缺失、时钟倒退、容量告警或来源权限不符合要求；
- 实际 unit、账户、capabilities、构建特性与审查记录不同。

保留日志、非敏感状态摘要和当前凭证，先恢复服务再定位。不删除 journal、信任缓存、根链
或 current；不盲目重跑 initialize、不降低 floor。未提交的首次迁移可由固定 root 恢复程序
恢复旧启动方式；备份不符则失败关闭，需要人工。已提交迁移或日常升级不能靠首次恢复单元
任意降级；失败目标也不会自动反复尝试。

换 root/targets 密钥、吊销、丢失保管人或需提高回滚下限时，使用
[签名规程](../tools/nodelite_release/README.md)另开维护窗口；root 阈值失陷或不可满足时，
必须独立分发新信任锚并本地确认，不能声称在线自动自救。
