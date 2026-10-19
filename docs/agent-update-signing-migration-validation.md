# 签名仓库、密钥管理与首次迁移验证（第六阶段）

日期：2026-10-19（Asia/Singapore）。承接[第五阶段](agent-update-rust-validation.md)。

## 本轮交付与边界

- `tools.nodelite_release`：加密密钥、独立审批、root 双阈值换钥、角色续签、静态仓库原子发布和 enrollment 组包。
- `tools.nodelite_migrate`：默认检查、显式本地 root 迁移、未提交事务的冷启动恢复和保留信任状态的 resume。
- Rust updater 增加 `verify-enrollment`，签名下载、信任缓存、防降级与日常激活继续使用 Rust 实现。
- 旧 Agent 安装脚本拒绝覆盖已启用 signed updater 的启动路径；普通手动安装不自动迁移。
- CI 配置增加操作工具回归，但本轮没有推送或运行 GitHub Actions，不能把云端测试称为 GitHub CI 全绿。

没有网页接口、Server 升级协议、生产签名源、生产密钥、tag 或发布；没有连接现网 HK、Oracle、
Korea 或其他节点。操作工具/规程见[签名说明](../tools/nodelite_release/README.md)和
[首次迁移说明](../tools/nodelite_migrate/README.md)。

## 签名与发布

真实 CLI 生成 Ed25519 加密 PKCS8 私钥，root 为 2-of-3，targets 为 2-of-2，snapshot 和 timestamp
各自独立密钥。审批绑定规范化签名载荷，重复 key ID 不增加阈值，轮换 root 必须同时满足旧、新
阈值。角色之间拒绝密钥复用；公开 key ID 根据实际公钥重算，不能伪造 ID 扩充票数。

发布器验证完整根链、所有角色签名、过期时间、引用长度/摘要、目标大小、渠道/架构/产品范围和
序号下限。核验后的字节快照被发布，而非再打开可变输入。先持久化不可变 generation，再原子
更新 current；已有版本不接受不同内容。一个仓库只服务一个渠道/平台。

元数据各角色版本独立：不改变 targets 的 timestamp 续签已由 Python 测试和实际 Rust 客户端
验证；snapshot 单独刷新也有回归覆盖。否则 12 小时 timestamp 会迫使操作者频繁动用两把离线
targets 密钥，这是本轮补齐的重要运维边界。默认有效期：root 365 天、targets 90 天、snapshot
7 天、timestamp 12 小时；没有在生产创建刷新定时任务。

所有实验私钥只在编译机临时 0700 目录生成，文件 0600、加密保存；随机口令仅通过 stdin FD
传入签名进程，不进入参数/环境变量/日志。正常结束清理临时目录。只有公开元数据、ELF 和固定
单元进入客机；这不是物理安全擦除、HSM 或真实独立保管人的证明。

## 首次迁移与 Token

迁移前检查默认安装、Agent >= v3.0.4、专属身份、精确配置权限、无 capabilities/流量控制/
自定义单元。Agent 没有 `--version`；采用 `--sample-once` 的版本身份，不输出整份采样。
包含 root 管理的 updater 与单元，因此要求独立核对**整包指纹和 root 指纹**，不能仅凭签名
Agent 的摘要信任其中的特权文件。

`prepared → stopping → switching → trial → committed`。签名、精确 baseline 比较和只读凭证
预检都先于停止旧进程。停机后 fsync 最新配置，切换固定单元，新程序真实鉴权上报并持续推进
健康时间后才提交。root 管理工具额外核对 `/proc/PID/exe`；日常 updater 没有这些特权。

root 私有目录只备份旧 unit 和程序，**不备份或恢复 Token/配置**。失败或未提交断电仅恢复旧
启动方式；根链、已见 release/元数据下限和最新 Token 保留。备份不符时要求人工，不能猜测
一份旧程序启动。已提交迁移不会被该恢复单元再次回退。

resume 只允许 rolled_back 的默认旧安装继续，保留首次 receipt；不能变更 root、updater、
固定单元、URL、渠道、架构或原 rollback floor。仓库已前进时可独立核对新包、更新 Agent
baseline，沿用原 bootstrap anchor 和所有已接受的信任状态。状态丢失不静默 initialize。
正式 Agent 的新组身份阻止 `verify-enrollment` 在迁移后改写 active 指针。

## 云端环境与结果

外层仍只使用 `compiler` 的 `~/NodeLite`，未安装宿主服务或使用宿主 root。
新实验客机位于 `output/agent-update-prototype/vm-enrollment-v1`，从旧实验盘复制后扁平化为
独立 24 GiB qcow2，旧 `vm-v1` 保持停止，不依赖会被继续写入的 backing disk。
客机 Ubuntu 22.04/systemd 249/cgroup v2/KVM；无宿主共享目录，网络 restricted。
Server、WS 代理和公开镜像只监听客机 loopback。

旧程序实际运行官方 v3.0.4 Agent/Server。候选使用上轮构建的真实 musl Agent
`rust-old`/`rust-new`，复用前核对 Agent/proto/Cargo 源码摘要，不是假冒新 release tag。
updater 重新编译为此环境的 native GNU 正常二进制，不启用 lab-faults。

| 验证 | 结果 |
| --- | --- |
| 真实迁移/签名/KVM 矩阵 | 17/17，包含 4 次强制断电冷启动 |
| Python 操作工具回归 | 19/19 |
| `cargo test --workspace --locked` | 663 通过、0 失败、3 项既有测试忽略 |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | 通过 |
| `cargo fmt --all --check` | 通过 |
| 原 Python 状态/缓存/时限模型回归 | 35/35 |
| 安装脚本回归及相关 shellcheck | 通过 |
| workflow 固定引用检查、示例凭证检查 | 通过 |

Rust workspace 命令使用 `NODELITE_SKIP_WEB_BUILD=1`，本轮未改网页、不代表执行前端构建或测试。

| 17 项矩阵分组 | 验证内容 |
| --- | --- |
| 默认迁移 | dry-run 不动服务；v3.0.4 → 签名健康基线；重复 apply 不重启进程 |
| 签名运营 | timestamp-only 续签不重启；root/targets 换钥并实际激活；吊销旧 targets 密钥后拒绝旧签名，再接受新签名 |
| 输入拒绝 | 错整包指纹、错 root 指纹、CAP_NET_ADMIN 均在停止前拒绝 |
| 可恢复错误 | 陈旧 Token、网络篡改目标被拒绝，旧 PID 保持；修复后 resume 不清空信任记录 |
| Token 回滚 | 新 Agent 鉴权后真实轮换 Token，再阻止健康写入；恢复的 v3.0.4 使用最新 Token 在线 |
| 包落后于仓库 | 旧 baseline 拒绝，保留已接受 root 2/release 2；新包沿原 anchor resume 到当前授权程序 |
| 迁移后边界 | 再次 verify-enrollment 被拒绝，active/状态/PID 不变 |
| 四次断电 | prepared、trial、trial 中 Token 落盘后、trial 中损坏旧 unit 备份后 |

断电通过 SIGKILL QEMU 后从同一磁盘冷启动，逐次核对 boot ID 变化。前三项恢复原程序并保持
最新配置逐字节不变；损坏备份时恢复单元失败，Agent PID 为 0，进入 manual_recovery，未使用
损坏备份启动。没有重启编译宿主或任何现网服务器。

## 实测调整

Cargo 的 release/deps 产物可能是硬链接，原来对所有输入的一律 nlink=1 检查会拒绝正常组包。
现仅公开 ELF 输入允许被别名，先快照再签名/校验；私钥、状态和节点配置仍拒绝多链接。
同时修正 systemd 249 省略空 ExecStartPre/Post 属性的兼容性，不放宽其他身份/单元要求。

root 恢复单元首次安装时不应与当前迁移锁相互等待；无 journal 时恢复为只读 no-op。
冷启动恢复不能同步等待被自身 Before/After 排序阻塞的 Agent，故恢复先持久化 rolled_back，
再通过固定 `start --no-block` 交给 systemd。上述时序由实际冷启动用例验证。

旧实验盘空间不足，采用独立扩容客机，没有清理用户文件。复制测试代码时避免保留 lab UID
所有权，迁移源代码由 root 重新拥有，满足 root 保护检查。最后将过长的实验签名生成函数拆分，
重新冻结源码并完整复跑，不沿用修改前的矩阵结果。

## 证据、收尾和仍未完成的生产准备

`enrollment-evidence.json` 需 complete=true 且恰好 17 项通过；`enrollment-build.json` 记录构建
前后不变的 107 个输入。`enrollment-audit.json` 另核对客机已安装的 12 个固定单元、正常 updater
映像与包一致，并确认无 lab-faults 标记。正常 updater SHA256：
`d5f1d6ec2aa781484d4059fcfe9ebb3e106c036f3de2e4fc4867c42a05fcc8c9`。

本地证据 `output/ops/agent-update-enrollment-20261019/` 被 Git 忽略，仅含非敏感 JSON/日志，
并再次比对本地源码和单元摘要。没有取回私钥、口令、Agent 配置或 VM 磁盘。
云端原始记录仍在 `~/NodeLite/output/agent-update-prototype/`，便于复查。

收尾停止、禁用客机服务并关闭 QEMU，检查 SSH 转发关闭、两个客机的 PID 文件均不存在、
编译宿主没有 nodelite/nlup-lab 单元。实验损坏夹具和私有盘保留在隔离输出目录；未删除用户数据。

当前不是生产自动更新上线：还需真实多方保管演练、生产 HTTPS 仓库/独立信任分发、角色刷新
值守与到期告警、长期公开仓库保留维护流程、发行版/架构矩阵及独立安全审查。
发布器现有 64 代次上限会安全拒绝继续发布，尚无自动 GC，不可绕过维护审查。
没有验证 HSM、可信时钟修复、磁盘控制器故障、任意 syscall 崩溃或主机 root 已失陷的情况。

后续[第七阶段安全审查](agent-update-security-validation.md)补充了两处核心修复、仓库
检查/显式归档和节点准入检查，并重新冻结源码完整复跑；本页结果和摘要保留为第六阶段
历史证据，不能用于替代新版产物校验。生产剩余条件见[受控部署准入](agent-update-deployment-gate.md)。
