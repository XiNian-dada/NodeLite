# 签名更新与首次迁移：差分安全审查

日期：2026-10-19。审查基线 `75e5000..b0f4ab0`，另复核本轮修复和运维工具。
这是代码审查、独立子代理对抗复核及隔离实验，不是第三方人工审计或绝对安全认证。

## 1. 摘要与准入结论

| 严重性 | 发现 | 修复状态 |
| --- | --- | --- |
| Critical | 0 | — |
| High / P1（条件性，开发中引入） | 1 | 归档路径 chmod 竞态已改为固定 inode 操作 |
| Medium / P2 | 2 | 长根链追赶阻塞、迁移切换恢复依赖缺口已修复 |
| 未修复的已确认安全缺陷 | 0 | 仅针对本次已审查范围，不代表不存在未知缺陷 |

整体固有风险 **HIGH**：更新器可替换 Agent 代码，首次迁移需要本地 root。
结论为 **CONDITIONAL**：冻结源码的最终矩阵已通过，可进入已验证平台的一台受控 canary；
生产独立密钥保管、信任指纹分发、HTTPS 仓库、角色续签和告警值守不是测试可以替代的事项。
本轮未授权全量上线，未接网页、未连接现网、未生成生产密钥、未推送或发布。

审查策略 FOCUSED：基线 12 个提交、81 个变更文件，+6687/-42 行。深读 16 个 Rust updater
源文件、12 个固定单元、10 个签名/迁移生产 Python 模块，以及健康证据与安装器边界；其余
测试、实验、文档、Cargo/CI 变更做用途和范围扫描。未测覆盖率百分比，不把用例数当行覆盖率。

## 2. 变更与历史依据

| 历史提交 | 关键变化 | 风险 |
| --- | --- | --- |
| `20d9b59` | Agent 本地鉴权健康证据 | 证据真实性/新鲜度 |
| `96111de` | Rust TUF 下载、激活、固定 systemd 边界 | HIGH |
| `c6d3e1e` | 中间 root 持久化和有界恢复状态 | 吊销保持、缓存一致性 |
| `df0ba01`、`f02e83d` | 多方审批、不可变发布、在线角色独立续签 | HIGH |
| `8a39118` | 显式首次迁移、root 开机恢复、resume | HIGH |
| `c10241c` | 旧安装器拒绝覆盖已迁移路径 | 防绕过 |
| `7cbcfcb`、`e37ed9d` | 真实二进制/KVM/断电矩阵 | 回归证据 |

`git blame` 确认每轮 8 个 root 的限制来自 `96111de`；先写 unit、后写恢复 drop-in 的
顺序来自 `8a39118`。二者并非本轮删除安全校验造成；持久化中间 root 后，较小的重放预算
暴露了跨模块活性冲突。原始安装器的凭证持久化路径不在本轮重写范围内。
本轮抽取账户检查时保留原限制，仓库原排他锁扩展为统一共享/排他锁，没有删除签名或权限检查。

## 3. 发现、可达场景与修复

### F-01 / P2：离线节点根链跨度超过 8，重试无法推进

位置：`nodelite-updater/src/download.rs:128`；历史 `96111de`。
直接入口 2 个：日常 download/apply 和 enrollment 验证均调用 `download::fetch`。

节点缓存仍以 root 1 为 TUF 引导，需要连续处理 root 2..11。原每轮配额 8，虽然已验证的
中间 root 被持久化，下次仍须从同一 bootstrap 顺序重放；再次耗尽预算，未到达 timestamp。
镜像不必伪造签名，正常长期离线节点就能触发。影响更新可用性，未导致签名绕过或旧进程离线。

隔离诊断连续执行两次 download，均 exit=1，持久化 root 1..9，release 仍为 1；PID 和
Token 不变。修复将预算设为 128，与持久根历史硬上限对齐；保留单请求 10 秒/整轮 45 秒
网络预算，仍通过同一 TUF 库重放，不能跳过根链、直接混用旧角色缓存。

回归：`security_root_gap` 使用真实审批生成 11 级根链，两次 download 都须成功、接受
release 2，且不重启旧进程、不更改凭证。超过 128 的历史仍需人工保留信任的维护方案，
不支持删除缓存重新初始化以绕过上限。

### F-02 / P2：首次迁移 unit 切换的断电窗口缺少失败关闭依赖

位置：`tools/nodelite_migrate/transaction.py:229`；历史 `8a39118`。
直接调用者 2 个：apply、resume 的 handoff。影响正在首次迁移的节点，不影响未迁移安装。

旧顺序先原子持久化新 Agent unit，再创建 Requires/After 恢复 drop-in。恰在前者完成后断电，
若原启动备份也损坏，恢复会进入 manual_recovery，但新 Agent 不依赖恢复成功而仍可启动。
这里候选已获签名，不能据此执行任意恶意包；问题是“需人工时不启动”的恢复保证被破坏。

Root 限定的实验 wrapper 在 unit 原子发布后 SIGSTOP，再损坏夹具备份、SIGKILL QEMU、
从同盘冷启动。修复前记录 recovery dependency=false、manual_recovery、Agent PID=707，
而非期望的 PID=0。该 hook 只存在实验代码，不进入发货程序。

修复先持久化固定恢复 drop-in，再写新 unit。回归 `security_switching_backup` 在同一边界
强制断电，必须有 durable dependency，恢复失败后 PID=0，最新 Token 不变。

### F-03 / 条件性 P1：新归档工具的权限恢复跟随可替换路径

位置：`tools/nodelite_release/maintenance.py:19`、`:30`。
仅存在于本轮开发中的未提交版本，首个提交 `40d1d4c` 已包含修复；不将其描述为现有 release 漏洞。
直接生产调用者 1 个：collect → retire；潜在影响以发布操作者可访问的文件为边界。

若归档祖先可被另一 UID 重命名，只校验叶目录 0700 不够。攻击者在检查后重定向归档路径，
再将已移动的随机条目替换为指向操作者私密文件的符号链接，原 finally 的 Path.chmod(0555)
可能改变私钥文件权限；后续 fsync 报错不能撤销泄露。Root 运行时影响可扩大到 root 文件。
这是有部署前提的路径竞态，不是已证明的远程攻击。

修复：archive、generations 和 generation 均持有 O_DIRECTORY|O_NOFOLLOW 打开的 FD，
校验 owner/mode；相对 dir_fd rename，fchmod/fsync 只作用于持有 inode；移动后 nofollow
比较 dev/inode。两个确定性 hook 用例覆盖 archive 条目替换和归档路径替换，私密夹具始终
保持 0600、原 generation 恢复 0555；独立子代理复核未发现原 chmod 劫持仍可达。
未运行真实多 UID 并发利用，不把 hook 回归冒称完整攻击复现。

仓库 lock/current/inventory 仍按路径读取，因此生产仍要求所有祖先不可被其他 UID 替换。
FD 修复不承诺在恶意可写祖先下抵抗所有 DoS 或维持原归档路径。恶意发布者自身属于 TCB。

## 4. 攻击者与影响面

| 身份/能力 | 审查结果与边界 |
| --- | --- |
| 被攻破的 Server/Web 或镜像 | 不能指定本地 URL/命令/服务名；签名、范围、摘要、有效期、下限仍是节点信任依据。可阻断更新，不能因此获得签名能力 |
| 其他非特权本地 UID | 不可改 root 策略/单元，不可读取 0700 Agent 凭证；路径链接/异常 inode 被拒绝。签名机还须保护所有祖先 |
| updater UID 完全失陷 | 可替换 Agent 代码，间接取得 Agent Token；这是声明的可信计算基，不声称 Token 与恶意更新器完全隔离。未发现由固定 bridge 扩展到主机 root 的路径 |
| Agent 自身失陷 | 可伪造自己的健康上报；本地健康不是远程证明，不排除既有主机/内核漏洞 |
| 满足 root 签名阈值的攻击者 | 已破坏信任锚，不能由同一在线链自动挽救；需事件响应和独立重新引导 |

量化的一跳影响：fetch 2 个入口；handoff 2 个入口；仓库锁 4 个生产调用（publish、pack、
check、collect）；Agent 健康发布 2 个采样发送分支。12 个固定单元包含 3 类 root bridge
动作（preflight/stop/restart），不执行 updater 可写脚本；服务主体仍是无 capabilities 的
Agent UID。新增 audit 仅由本地 root CLI 调用，未增加 Server API 或任何网页/协议命令。

## 5. 补齐的上线工具

- `check --root-sha256`：从独立指纹锚验证完整公开仓库，分角色到期/容量告警，exit 2 可接值守；
  不会自动续签，不代替在线签名密钥管理。
- `collect`：默认计划、显式应用、同盘私有归档、共享读锁/排他写锁、最低保留与宽限，不删除历史。
  当前代次仍保留公开历史，长期字节数增长和归档清理仍由管理员管理。
- `tools.nodelite_migrate --audit`：核对 receipt、root/updater/12 单元摘要、恢复依赖、账户、
  配置 inode/权限、updater 终态和实际执行映像/近期健康；不读 Token、不改服务或凭证。
  status 可能创建锁或维护 journal 标记，不能称为绝对零状态写入。
- [受控部署准入](agent-update-deployment-gate.md)：独立密钥、生产源、告警演练、单节点
  迁移/更新/重启和停止推广条件。没有把实验密钥、HTTP loopback 或故障构建变成生产默认。

## 6. 测试与证据

以下均为完成后的结果；未使用正在执行或源码有变动的矩阵充当通过。

| 检查 | 已确认结果 |
| --- | --- |
| Rust 升级 KVM 矩阵 | 27/27，7 次强制断电，真实磁盘字节/inode 耗尽 |
| 首次迁移 KVM 矩阵 | 17/17，4 次强制断电，真实 Token 轮换/回滚/resume |
| 新增安全准入矩阵 | 10/10，另 2 次强制断电，源码快照一致 |
| Python 签名/迁移/运维回归 | 27/27，含 2 项路径替换回归 |
| 工作区 `cargo test --workspace --locked` | 663 通过，3 项既有忽略，0 失败 |
| workspace all-targets/all-features clippy `-D warnings` | 通过 |
| `cargo fmt --all --check` | 通过 |
| 原 Python 状态/网络/缓存模型 | 35/35 |
| 安装脚本回归、相关 shellcheck | 通过 |
| 固定 Actions 引用、示例凭证检查 | 通过 |
| OSV package/version 查询 | 本次查询 222 项无已知漏洞返回，不代表无未知漏洞 |

OSV 范围为本机 Linux updater 的 Cargo resolve 子树（保守包含开发依赖）及 7 项锁定 Python
依赖，不是所有平台/Server 全依赖漏洞审计。首次 offline metadata 因平台缓存缺项失败，
改为联网读取公开 Cargo metadata 后完成；未上传源码、密钥或配置。

实验只在 compiler 的 `~/NodeLite` 下执行，使用独立 24 GiB KVM 客机，Ubuntu 22.04、
systemd 249、cgroup v2，无宿主共享目录。旧 vm-v1 保持停止；所有 Server/代理/镜像仅客机
loopback。正常 native GNU updater 与 musl Agent 实际互操作，故障 updater 使用不同摘要。
Rust workspace 使用 NODELITE_SKIP_WEB_BUILD=1；不声称网页测试、GitHub CI 或全架构验证通过。

修复前 `security-probe.json` 明确 complete=false/probe=true。诊断时其他操作工具源码同步
导致 build_snapshot_matches=false，因此它只支持定位（核心旧下载/迁移代码未变），不是
发布验收。最终矩阵必须在源码冻结后完整重跑，不能拼接之前未完成的 17 项矩阵。

最终 `rust-matrix-evidence.json`、`enrollment-evidence.json`、`security-evidence.json` 均
complete=true，分别恰好 27、17、10 项通过；后两项使用同一份 116 个输入的冻结快照。
本地再次逐一 SHA256 比对成功。修复后长链为 root 1..11，两次下载 exit=0、接受 release 2；
unit 边界依赖已持久化，坏备份冷启动为 manual_recovery/PID=0。最后的 committed 冷启动
不回滚，实际 `--audit` 返回 ready；篡改固定单元后 audit 拒绝。

正常 updater SHA256：
`29618164ac001076c6fc88976be7003583d57d84dd25d13a4d0b492e7d0c58e6`。
它是本次 native GNU 实验构建的摘要，不是官方发布产物。故障二进制摘要不同；收尾逐项核对
guest 的 12 个固定单元与首次 receipt，正常 updater 无故障标记字符串，配置仅 stat 权限、
未读取内容。停止并禁用 guest 服务，无遗留故障 drop-in/实验存储挂载，两个 VM 均停止，
22239 转发关闭，编译宿主没有 nodelite/nlup-lab 单元。未删除用户文件，实验盘和夹具保留。

本地非敏感证据位于 Git 忽略的 `output/ops/agent-update-security-20261019/`，包括原始
矩阵、源码比对、测试日志、依赖查询及 `security-final-guest-audit.json` /
`security-final-host-audit.json`。未取回密钥、口令、Agent 配置、数据库或 VM 磁盘。

未覆盖：真实多方离线保管/HSM、生产 CDN/TLS/告警值守、aarch64、其他发行版/SELinux、
真实硬件断电/控制器缓存错误、任意 syscall 切点、可信时钟修复、主机 root 失陷、恶意签名
二进制和虚拟机快照整体回滚。对已测路径信心高，对全部运行环境的推断信心中等。

## 7. 生产前阻断项与后续维护

代码修复必须随其回归一同交付。生产独立密钥/指纹分发、只读 HTTPS 命名空间、在线角色
调度与告警演练、canary 维护窗口仍是部署阻断项，不擅自替管理员完成信任决策。
其他平台维持手动升级，不为了兼容开启 updater sudo、capabilities 或任意远程命令。
发生 manual_recovery 不删除 journal/缓存，Token 和信任状态不得回滚。

具体执行顺序和失败条件见[部署准入规程](agent-update-deployment-gate.md)。
本轮建议不是“保证不会被黑”，而是对明确环境和威胁边界的有条件准入。

## 8. 参考与方法

按 differential-review 的风险优先差分方法执行：Git 历史和 blame、一跳调用影响、关键文件
深读、攻击场景、独立子代理对抗复核、旧代码诊断、修复后的真实隔离回归。未对整个 Server
认证、Web 或历史 Token 修复重新做全仓审计，未宣称 81 文件逐行等深度检查。

[TUF 规范](https://theupdateframework.github.io/specification/v1.0.36/) 用于交叉核对连续
root 的旧/新阈值验证和分角色有效期；实现仍使用 tough/python-tuf，而非自行实现密码学。
[Tough 官方历史安全通告](https://github.com/awslabs/tough/security/advisories/GHSA-5vmp-m5v2-hx47)
用于检查根链相关回归背景，锁定版本为 tough 0.24.0；
[OSV API](https://google.github.io/osv.dev/api/) 用于本轮已知漏洞查询。

本轮本地检查点：`40d1d4c` 仓库检查/归档；`345e2af` 固定实验 VM 选择；`a5987da` 根链
重放预算；`8b029ae` 迁移失败关闭/准入；`9365ee7` 安全回归矩阵。没有推送、tag 或 release。
