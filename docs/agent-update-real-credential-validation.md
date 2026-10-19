# Agent 真实凭证交接验证（第二阶段）

日期：2026-10-19（Asia/Singapore）。

后续正式产物、完整 Server/真实 token 断电及参考 TUF 网络测试见
[第三阶段验证](agent-update-release-network-validation.md)。本页保留第二阶段的原始范围和结果。

实现检查点：`7886da1` 停止屏障，`0e980c1` 健康证据推进，`d5e5a8e` 真实凭证测试，
`80aebd2` 停止完成确认与冷启动动作恢复，`b0132ef` 复现入口和运行证据约束。

## 范围与结论

在编译机 `compiler` 的 `~/NodeLite` 内构建，在原有隔离 KVM 客机内测试。
没有接网页、发布、推送或操作现网；没有复制真实节点凭证、SSH 私钥或数据库。

这一阶段补上了实际 Agent 的配置写盘、WebSocket 认证、token 轮换/ACK、程序交接与回退。
两份 ELF 是同一份应用源码分别注入 `lab-old` / `lab-new` 构建标签，**不是已证明任意
两个正式发行版兼容**。Server 使用生产 WS/registry/refresh 处理链的 `TestServer`，
不是完整生产 CLI 和后台任务部署。签名仍是客机内一次性测试密钥，不是生产 TUF 信任链。

## 真实 Agent 场景

11/11 通过；每笔恢复都检查真实鉴权和新采样，并用新加载的注册表验证磁盘 token。

| 场景 | 观察点 |
| --- | --- |
| `permissions` | 实际 Agent UID、零有效 capability、NoNewPrivs；updater 无直接配置读取权限 |
| `stale_disk` | 磁盘凭证失效但 RAM 会话健康：预检拒绝，原 PID 和上报不被替换 |
| `rotate_upgrade` | 轮换成功落盘，升级后新进程使用磁盘凭证鉴权上报 |
| `rotate_rollback` | 新程序再次轮换后卡死：回退程序，最新配置字节不回退，旧程序仍能鉴权 |
| `write_rejected` | 配置目录不可写时不确认持久化，不替换原凭证；恢复权限后可升级 |
| `lost_ack` | 新凭证已落盘，ACK 被中继截留后 SIGKILL；接任进程用持久候选恢复 |
| `killed_before_delivery` | 轮换响应送达前 SIGKILL，原磁盘凭证仍可重连并升级 |
| `refresh_vs_stop` | 已落盘但 ACK 未送达时停止旧服务并升级，不发生新旧进程交叉写盘 |
| `stopping_crash` | 协调器在 STOPPING 记录后被杀，幂等继续交接 |
| `trial_journal_crash` | 旧服务已退出、TRIAL 落盘但指针未切换时被杀，恢复登记基线 |
| `pointer_crash` | 指针已切换但启动请求未发出时被杀，恢复登记基线 |

为了控制消息边界，客机 loopback 中继只截留响应或 ACK，不打印消息体。
测试控制器使用 stdin 传配置，凭证不进入命令行参数。健康适配由**可信测试 observer**
将真实进程摘要/启动实例与 Server 新采样关联；它不是生产健康证明或 Agent IPC。
测试控制器以客机 root 运行，但它不属于 updater 被授权执行的 root 服务入口。

## 发现并修复的原型问题

### 单次采样被当成持续健康

新版在首次上报后立即卡死，缓存心跳仍在有效期内，原来的稳定窗口可能提前提交。
现要求单调时钟的稳定时长与证据采样时间都持续推进；重复轮询同一份采样不能刷新证据。
未来/陈旧采样或 invocation 改变会重置窗口。修复后真实卡死场景回退成功，配置保持最新。

### 迟到或跨 boot 的停止请求

对未消费 stop 标记进行断电，并给 root 动作注入 2 秒启动延迟，复现了恢复后的旧程序
再次被停掉：journal 已回到基线，但服务最终为 inactive。

修复后固定 root stop 使用同步 `systemctl stop`；协调器既等待该动作的新完成记录，
也检查请求已消费、Agent inactive、MainPID=0 和空 cgroup。不能仅凭暂时没有主进程
就切换。停止状态无法确认时进入人工恢复，不猜测并启动另一程序。
各 bridge path 同时声明 boot 的启动依赖与执行顺序；boot 持锁清掉三个固定动作标记、
同步目录，再根据 journal 重建动作。这样旧 boot 的临时请求不会在恢复后重放。
实测还暴露了 `.path` 默认排在 `paths.target` 之前、boot service 排在 basic 之后的
隐式依赖环：只加 After/Requires 不够。bridge path 使用显式晚启动依赖并保留 shutdown
冲突/顺序，安装时先检查完整 unit 图，避免依赖环让 systemd 跳过恢复任务。

该依赖与顺序的区分见 [systemd.unit](https://www.freedesktop.org/software/systemd/man/systemd.unit.html)，
路径触发语义见 [systemd.path](https://www.freedesktop.org/software/systemd/man/systemd.path.html)。
实际兼容性以本客机 systemd 249 的运行和单元检查为依据，不以最新手册代替实测。

## 冷启动故障复跑

6/6 通过：PREPARED、STOPPING、残留 stop 请求、TRIAL 已落盘而指针未切换、
active 已切换五个检查点强制 SIGKILL QEMU，以及损坏 journal 的启动拒绝。
前五项恢复登记基线并确认配置未变化；随后持续观察超过注入的 2 秒延迟，防止只检查到
短暂恢复便漏掉迟到的停止动作。这一组使用 ELF 夹具，不冒充真实 token 的全链路断电测试。

## 验证命令与证据

云端 `cargo test --workspace --locked`、`cargo clippy --all-targets --locked -- -D warnings`、
`cargo fmt --all -- --check` 通过。后端构建显式设置 `NODELITE_SKIP_WEB_BUILD=1` 并复用
已有 dist；没有声称运行本次前端构建/测试或 GitHub CI。

17/17 离线 Python 回归通过（9 项文件 I/O、8 项协调器/启动边界）。
完整 ELF 夹具回归 28/28 通过；13 个固定 systemd unit 的检查通过。
VM-only Rust 测试默认 ignored，正常 workspace 测试不会在编译宿主上安装单元。
复现入口会确认精确测试名存在、清理旧结果，再运行客机测试，避免零测试成功退出冒充通过。

已取回的非敏感证据放在本地忽略目录 `output/ops/agent-update-real-20261019/`：

| 文件 | SHA256 |
| --- | --- |
| `real-evidence.json` | `66c5f3984ac3b2dbf8ca74bcce52067f61984f387392b73b9cf06a3e8774e197` |
| `evidence.json` | `42adfbc756e7c679a488661f7e6cbf5422f6fab3c8cae92a6029657905a27322` |
| `powercuts.json` | `7e5d99b2452e5d5393777d59f3fd01ff45af9eb38c0f5f26996cb3ed865e789b` |
| `python-final.log` | `6a22518e6b1806e0b45a46929f165341318b9f42326c32f577d98e7a8770a6aa` |

两套运行证据中的实际 worker 摘要一致，并与本地提交的 `updater.py` / `lab_io.py` 一致。
客机仍为 KVM、内核 `5.15.0-198-generic`、systemd `249 (249.11-0ubuntu3.22)`。
QEMU 已关闭，编译宿主没有 `nlup-lab-*` 单元，转发端口 22239 没有监听。
私有客机磁盘和临时测试密钥仅保留在云端项目 output 以便复现；未取回私钥、配置原文或磁盘。

复现入口：[实验说明](../experiments/agent-update-isolation/README.md)。
设计契约：[目录权限与状态机](agent-update-isolation-prototype.md)。

## 仍未完成

- 完整 TUF、换钥/吊销、可信时间、独立发布授权和实际 GitHub/代理下载。
- 生产 Rust 更新器、事件/期限驱动调度、生产 Agent 本地健康/能力协议。
- 不同正式版本组合、首次可信基线迁移和配置格式兼容策略。
- 完整生产 Server 重启、真实 token 全链路断电；本阶段真实凭证故障主要是进程/消息边界。
- 任意 syscall 内部中断、真实满盘/inode 耗尽、存储控制器错误、发行版矩阵和性能预算。
- 独立安全审查、部署 opt-in、管理认证/审计和最后的网页入口。

通过这些场景不等于可以给现网启用自动更新，更不构成清除已失陷节点的能力。
