# Agent 更新隔离原型（草案 v1）

状态：仅验证 Linux/systemd 的权限、编排与故障恢复，不是可安装到生产的更新器。
不修改现有 Agent、协议、安装脚本或网页。所有程序、凭证、签名密钥均为测试夹具。

## 验证范围和信任边界

原型只在云端编译机 `~/NodeLite` 下的一次性 KVM 客机中运行。客机无宿主目录共享，
网络使用 QEMU restricted user networking，仅向编译机 loopback 转发 SSH。
不在宿主机安装服务、创建用户、配置 Polkit/sudo 或初始化 LXD；不连接现网面板。
客机安装入口同时要求专用标记文件和 QEMU/KVM 虚拟化检测。

| 对象 | 所有者/权限 | Agent | updater |
| --- | --- | --- | --- |
| `/usr/local/lib/nlup-lab/` | root，目录 0755，代码 0555 | 执行 | 执行，不可改 |
| `/etc/nlup-lab/` | root，目录 0755，测试公钥/策略 0644 | 读公钥 | 读，不可改 |
| `/var/lib/nlup-lab/code/` | updater:`nlup-read`，0750 | 只读/执行 | 管理对象和 active |
| `code/objects/<sha256>/agent` | updater:`nlup-read`，目录 0750、程序 0550 | 执行 | 验签后发布 |
| `/var/lib/nlup-lab/private/` | updater，0700；记录 0600 | 不可访问 | 事务、锁、恢复状态 |
| `/var/lib/nlup-lab/agent/` | Agent，0700；配置 0600 | 读写 | 无直接访问权限 |
| `/var/lib/nlup-lab/inbox/` | Agent，组允许 updater 读取/消费 | 不可信请求 | 有界导入 |
| `/var/lib/nlup-lab/bridge/` | root:updater，0770 | 不可访问 | 创建固定动作标志并同步目录 |

这些目录的父目录由 root 管理，普通用户不得替换祖先目录。对象区和私有状态区分开。
updater 属于可信计算基：如果它已被完全控制，可通过控制 Agent 程序间接获取 token，
双 UID 不提供反方向的凭证保密保证。首版排除所有额外 capabilities。

## 固定单元职责

| 单元 | 身份 | 职责 |
| --- | --- | --- |
| `nlup-lab-agent.service` | Agent | 从固定 active 路径运行测试探针，空 capabilities |
| `nlup-lab-preflight.service` | Agent | 候选无密钥离线自检，不读真实配置 |
| `nlup-lab-request.path/service` | updater | 消费 inbox 并校验，不直接获得服务管理权 |
| `nlup-lab-reconcile.service/timer` | updater | 同一个幂等协调器处理试运行期限及失败恢复 |
| `nlup-lab-boot.service` | updater | 在 Agent 开机启动前恢复未完成事务 |
| `nlup-lab-restart.path/service` | root | 消费固定 marker，固定 `reset-failed` 和 `restart` |

root 单元只执行系统自带的固定命令；不运行 updater 可写的程序、脚本或环境文件。
无模板实例参数、任意 unit 名、`StartTransientUnit`、Polkit 或免密 sudo。
`.path` 不是安全队列：请求有大小/类型限制，任务 ID 去重，状态持久化，协调器有互斥锁。
只有 updater 私有的 bridge 路径能唤醒 root 动作，Agent 不能通过 inbox 直接重启服务。
`OnFailure` 只唤醒 reconcile，不能无条件回滚或无限清零启动限流。

## 文件发布与事务

`active` 是可重建的指针，私有 journal 才是事务真源；不依赖双软链原子交换。
对象按内容摘要命名。导入不可信文件时，拒绝软链、硬链接、FIFO、设备、超限文件；
复制到 updater 新建的私有 inode 后验签，不直接 rename/chown Agent 提供的 inode。

```text
STABLE -> PREPARED -> TRIAL -> COMMITTED
                         -> ROLLBACK_INTENT -> ROLLED_BACK
不一致或无法安全恢复 -> MANUAL_RECOVERY
```

journal 包含 schema、代次、job ID、from/to 摘要、发布清单摘要/序号、阶段、
rollback_consumed、boot ID、试运行单调时钟期限和非敏感结果码；不含 token/配置副本。

1. 完整写入目标对象并同步文件、对象目录和父目录。
2. 同步 PREPARED；旧 Agent 完成凭证检查及持久化交接屏障后，才允许切换。
3. **先同步 TRIAL，再原子替换 active，最后同步 active 父目录。**
4. 正式 unit 启动新版，核对 unit 的启动身份/实际执行映像、稳定运行及功能证据。
5. 成功后同步 COMMITTED；之后才清理非必要对象。
6. 回退前先持久化 ROLLBACK_INTENT 并消费一次逻辑回退额度；幂等恢复唯一 from。

文件系统与 systemd 无共同事务，因此允许同一恢复动作的有界幂等重做，
不承诺物理 `systemctl` 恰好调用一次。已提交事务永不再授权降级。
PREPARED 取消；TRIAL 在重启后根据 active/from/to 摘要恢复；无法匹配时转人工。
旧 boot 的单调时钟不能跨重启使用。失败目标不能自动重新发起同一次升级。

## 预检、健康与 token

候选只执行无真实 token 的离线测试。当前 Agent 负责验证当前磁盘凭证、停止接收新的
轮换并等待已有写盘/ACK 边界；进程内 CONFIG_IO_LOCK 不视为跨进程锁。
正式 Trial 才验证新版真实配置解析和鉴权兼容性；不同时运行两个正式节点会话。
只读 mount 防常规误写，不作为恶意同 UID 进程的完整隔离。

客机夹具用随机测试配置和可观测心跳模拟进程/凭证持久化，不声称已证明真实 WebSocket
轮换正确。须额外集成实际 Agent/Server 测试后才能生产化。任何回退都不恢复配置/token。
远端不回执至多触发本事务允许的一次有限回退，不代表确定是坏程序。

## 签名与安全底线

原型使用客机内临时 Ed25519 测试密钥，覆盖产品/组件/平台/渠道、序号、版本、摘要、
长度和有效期。只证明本地受限导入/验证闸门，不称为 TUF 或生产发布授权。
生产方案另行评估成熟 TUF 客户端、持久 datastore、换钥、吊销和发布授权隔离。
`issued_at` 不代替软件版本/元数据序号；元数据高水位不能随二进制回退。
已生效的 hard floor 与成功提交后的安装基线分开。from 低于 hard floor 时拒绝可逆升级。
当前无新签名链的旧 Agent 不能自动成为可信回退包；首次启用须人工安装/登记基线。

## 原型验收

- 实际 UID/DAC、沙箱和固定服务边界；Agent 无法写代码、journal、bridge，updater 无法直读配置。
- 预检拒绝、无效签名/摘要、软链/FIFO/硬链接、超限包、降级/重放均不停止旧探针。
- 正常提交、坏可执行文件、秒崩、卡死、启动限流、并发/重复触发。
- updater SIGKILL、每个关键持久化/切换边界中断，恢复幂等且只允许登记的 from。
- 客机 QEMU 硬断电重启；journal 损坏和对象缺失必须转人工而非猜测。
- 试运行/回退后配置字节保持最新值，不输出测试凭证明文。

PASS 仅表示已记录场景在指定 guest/kernel/systemd 下通过，不等于生产可发布。
未验证的完整 TUF、真实 token 并发、资源故障和发行版兼容性必须保留为未完成项。

参考：[systemd.path](https://www.freedesktop.org/software/systemd/man/systemd.path.html)、
[fsync](https://man7.org/linux/man-pages/man2/fsync.2.html)、
[TUF](https://theupdateframework.github.io/specification/latest/)。
