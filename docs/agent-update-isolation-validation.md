# Agent 更新隔离原型验证记录

日期：2026-10-18（Asia/Singapore）  
原型提交：`f7e90f8`；设计草案起始提交：`c12755b`。  
结论：本阶段权限、固定 systemd 编排与选定故障恢复场景通过；**不是生产更新功能的交付或安全认证**。

## 已定下的草案

- [目录权限、固定单元和事务契约](agent-update-isolation-prototype.md)。
- [隔离运行说明与复现命令](../experiments/agent-update-isolation/README.md)。
- 固定单元的确切定义：`experiments/agent-update-isolation/setup_guest.py`。
- 状态机实现：`experiments/agent-update-isolation/updater.py`。
- 测试程序为独立 Rust ELF；协调器为实验性 Python，不进入现有 Rust workspace。

实现保留双普通 UID：监控用户不能写程序、事务和 bridge；更新用户无直接配置读取权限。
复杂的请求解析、验签、文件处理和恢复在更新 UID 下运行。root 固定单元只使用系统
`rm`、`systemctl` 的确定路径、参数与服务名，没有自定义 root 更新解析器或通用 sudo/Polkit。

程序以摘要寻址，单个 active 指针配合持久 journal；切换前记录 TRIAL，回退前记录
ROLLBACK_INTENT，开机恢复与失败/超时共用协调器。终态开机同样验证 active 与登记对象。
只回退登记的程序，不恢复配置。更新 UID 本身仍属于可信计算基。

## 隔离与环境

| 项目 | 实测值 |
| --- | --- |
| 云端位置 | 编译机 `compiler` 的 `~/NodeLite/experiments/agent-update-isolation` |
| 隔离方式 | 普通用户启动 KVM/QEMU 客机，无宿主目录共享 |
| 客机资源 | 2 vCPU、1 GiB RAM、8 GiB 稀疏 overlay |
| 客机内核 | `5.15.0-198-generic` |
| 客机 systemd | `249 (249.11-0ubuntu3.22)` |
| 网络 | restricted user networking，仅宿主 loopback SSH 转发；关闭 SSH agent forwarding |
| 宿主变更 | 仅项目内下载/解包工具和保存镜像；不安装宿主软件包、服务、用户或权限策略 |
| 宿主检查 | `nlup-lab-*` unit 清单为空；测试结束后 SSH 转发端口无监听 |
| 测试结束状态 | QEMU 已关闭；私有磁盘/测试密钥留在云端项目 output 以便复现 |

Ubuntu 镜像通过官方 HTTPS 下载和 SHA256 清单校验；这不是独立发布签名验证。
未连接现网节点或面板，未复制真实凭证/数据库，未修改 Agent、Server、协议、安装脚本或网页。

## 结果

| 验证组 | 结果 | 覆盖 |
| --- | --- | --- |
| 客机集成场景 | 26/26 | 真实 UID/DAC、权限拒绝、正常提交、无效包、预检与故障恢复 |
| 文件 I/O 回归 | 9/9 | 不可信文件类型、可写 FD 别名、umask、JSON 重复字段、fsync 故障注入 |
| 强制断电与启动闸门 | 4/4 | 三个事务检查点硬停止 QEMU，及损坏 journal 的冷启动拒绝 |
| systemd 单元检查 | 通过 | 11 个固定 unit 的 `systemd-analyze verify` |
| Rust fixture | 通过 | 云端 release build、fmt check、cargo test、clippy `-D warnings` |

`cargo test` 是独立 fixture 工作区的编译/测试目标检查（其中没有额外 Rust 单元测试）；
真实 ELF 行为由上述客机集成场景覆盖。没有声称已跑 NodeLite 全量 Rust/前端 CI。

26 个客机场景：

```text
permissions, success, preflight_fail, bad_executable, crash, hang,
bad_signature, bad_digest, symlink, fifo, hardlink, oversize,
expired, wrong_component, downgrade, replay, config_advanced,
kill_prepared, kill_trial, kill_pointer, kill_rollback,
concurrent, sequential, missing_object, unexpected_active, manual_recovery
```

`sequential` 在不重置基线的情况下完成三笔事务：成功升级一次，然后两次失败回退；
用来检查恢复额度等状态不会错误地跨任务继承。`config_advanced` 在 Trial 中修改测试配置，
回退后逐字节检查最新配置未被旧副本覆盖。该配置是随机夹具，**不是实际 NodeLite token**。

四项冷启动验证：

1. PREPARED 已持久化，未切换 active：强制断电后取消准备，原程序可用。
2. TRIAL 已持久化，active 仍为原程序：强制断电后安全回到登记基线。
3. active 已切到候选：强制断电后恢复唯一登记的 from，并核对配置未变化。
4. journal 损坏：冷启动进入人工恢复，Agent 不执行无法确认的 active 对象。

## 原型中发现并修正的问题

- 未加载的预检 unit 上 `reset-failed` 会返回失败，不能因此跳过固定启动动作。
- oneshot 预检结果需要保持可关联；用固定 unit 的退出时间关联，不能依赖已清空的 InvocationID。
- 私有 umask 会去掉新对象的组执行位；在新 inode 上显式设置最终模式，并补回归测试。
- bridge 目录只有写/遍历权限时不能打开以 fsync；调整为 root:updater 0770，Agent 仍不能访问。
- 每笔新事务必须清除上一笔 `recovery_requested` 等运行状态，避免连续失败时无法恢复。
- 终态也必须核对 active/对象，不能因 journal 写着 COMMITTED 就跳过冷启动检查。
- 测试夹具重建原对象时必须恢复共享组，否则会把实验准备错误误判为回滚缺陷。
- SIGKILL 后的 QEMU 写锁释放可能稍有延迟；有界等待，不关闭磁盘锁或并发写 overlay。

## 证据

已取回的非敏感证据位于本地忽略目录 `output/ops/agent-update-prototype-20261018/`：

| 文件 | SHA256 |
| --- | --- |
| `evidence.json` | `be4ae0be35578376652ec68fe44ba245be7d719700f67ae24df3dcf173dd8988` |
| `powercuts.json` | `996e6f007e882b701cf5c7e73f89034343ed568bec23f95959583cb3e644fd16` |

客机实际安装的 `updater.py`、`lab_io.py` 摘要与提交中的源文件一致。证据仅包含公开
权限/单元定义、场景结果和程序摘要；未取回测试私钥、配置原文或 VM 磁盘。

## 未完成的生产化验证

1. 将当前 Agent 的真实 token 写盘、轮换、ACK、交接屏障和回退后的握手纳入集成测试。
2. 成熟 TUF 客户端选型、持久 datastore、可信时间、换钥/吊销和独立发布授权。
3. 实际网络下载、代理/镜像策略、兼容性清单与首次可信基线迁移。
4. 真正满盘/inode 耗尽、存储控制器错误、每个系统调用内部的任意中断；当前只有
   指定 fsync 故障注入和选定 durable 检查点的客机断电。
5. 多发行版/systemd 版本、持续资源开销、事件驱动/期限驱动优化和独立代码安全审查。
6. 最后才接 Server/Agent 能力协商、管理认证、审计和网页按钮。

原型通过不代表可以给现网开启更新权限，更不代表能清除已经失陷的 updater/Agent。
