# Rust 签名下载到激活验证（第五阶段）

日期：2026-10-19（Asia/Singapore）。承接[第四阶段](agent-update-resilience-validation.md)。

## 交付范围

新增 workspace 成员 `nodelite-updater`，签名、持久缓存、状态机和激活均运行 Rust 正式代码。
仅测试发布仓库生成器和实验控制器使用 Python；不再用 Python 协调器替代待测激活逻辑。
现有 Server、网页、安装脚本和现网节点没有迁移或启用此功能，也没有推送、打 tag 或发布。

正常 updater 无故障注入入口；另一个显式启用 `lab-faults` 的二进制仅在专用客机暂停进程，
用于在确定的已持久化边界杀掉 QEMU。测试用的可写 tmpfs drop-in 不属于正式单元。

Agent 增加默认关闭的本地健康证据，WebSocket 协议未改变；它只在已鉴权会话成功发送采样后
发布实际采集时间、PID、invocation 和自身映像摘要，不包含配置或凭证。
独立更新器和固定 systemd 设计见[组件说明](../nodelite-updater/README.md)。

## 环境与判定

外层仅使用 `compiler` 的 `~/NodeLite` 编译与保存日志。固定单元、用户、loop 挂载均仅在
带标记的 KVM 客机内安装。没有连接 HK、Oracle、Korea 等现网节点，没有宿主 root 单元或挂载。
客机没有宿主目录共享，HTTP/WS/镜像仅监听客机 loopback。

实际运行官方 v3.0.4 Server CLI，两版本提交源码构建的 Agent 分别标为 `rust-old`/`rust-new`，
不是两个已经发布的 tag。最终 Agent 和故障 ELF 夹具实际交叉编译为 x86_64 Linux musl；
独立 updater 编译为此 Ubuntu 环境的 native GNU 二进制。没有声称验证 aarch64 或其他发行版。
链接器不支持 static-pie，Rust 回退到 static 的警告有保留，没有把这批实验产物当作发布资产。

发布实验使用参考 python-tuf 生成标准 metadata，root 2-of-3、targets 2-of-2。
所有签名私钥只在编译机实验进程内存中生成，不上传客机；这不是多个独立签名保管人的证明。
仓库通过真实 HTTP 提供完整 Agent 字节，Rust 端使用固定 `tough 0.24.0` 验证签名链。

## 最终结果

冻结源码后的完整重跑已通过，不沿用前几轮失败用例的结果：

| 验证 | 结果 |
| --- | --- |
| Rust 下载/激活/KVM 矩阵 | 27/27，包括 7 次 QEMU 断电冷启动 |
| `cargo test --workspace --locked` | 662 通过，0 失败，3 项既有测试忽略 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| Python 离线回归 | 35/35 |
| 原参考 TUF HTTP 回归 | 16/16 |
| 12 个正式 systemd 单元 | `systemd-analyze verify` 成功；仅客机既有 snapd 的未知配置键警告 |

Rust updater 另以 `--all-features` 运行了 14 项单元测试。上述 workspace 命令使用
`NODELITE_SKIP_WEB_BUILD=1`，不代表本轮执行过前端构建、前端测试或 GitHub CI。

## 端到端实验矩阵

| 分组 | 覆盖 |
| --- | --- |
| 正常切换 | 真实 Token 轮换后，签名下载、只读预检、停止旧进程、新进程健康推进、COMMITTED |
| 权限 | updater 不能直接读取 Token、Agent 不能写代码/状态/bridge、updater 无 sudo、程序无文件 capabilities |
| 拒绝下载 | 坏包、过期、错误产品、targets 签名不足、root 旧/新签名阈值不足、root 503、持续慢速 timestamp |
| 失败恢复 | 预检失败不停止旧 PID；秒崩、无心跳和试运行中轮换 Token 后冻结健康均回滚 |
| 持久信任 | 下载失败后拒绝旧 release；部分刷新后不遗忘被吊销的 targets 密钥 |
| 状态损坏 | 损坏 journal 后拒绝继续、不猜测或重置，原健康 Agent 保持运行 |
| 存储故障 | Rust 工作区真实满盘、inode 耗尽、元数据可提交但目标无法落盘；释放后重试 |
| 七次断电 | root 刚持久化、清单已提交、产物已持久化、PREPARED、STOPPING、TRIAL journal、active 指针 |

试运行 Token 测试先让真实新版鉴权，再通过正式 Server API 轮换并确认落盘，然后冻结其健康
输出以触发回滚。恢复的旧程序必须保留新版最后写入的配置，不能把 Token 随程序回滚。
用于秒崩/无心跳的故障 ELF 特意让夹具预检返回成功，以验证预检之后仍须经过真实健康闸门；
它不是实际 Agent 的凭证校验实现。
正式试运行不并行运行两个节点会话；失败目标不能因为重复 apply 自动再试一次。

断电通过 SIGKILL QEMU、从原磁盘冷启动实现，核对新 boot ID。三个下载阶段恢复旧 Agent，
已接受的 release 下限不丢失；激活阶段恢复到授权的 from，最多消费一次逻辑回退额度。
root 持久化后、尚未进入 timestamp 的位置另验证隐藏镜像的新 root 文件、使用旧 targets 密钥
仍被拒绝，随后由新密钥签署的发布可完成激活。

满盘实验使用独立 64 MiB ext4 noexec 普通文件 loop 镜像，仅绑定 updater 数据目录。
旧进程继续执行已映射的程序，Agent 配置目录不在被填满的卷内。分别确认 errno 28 和零可用块/
inode，旧 PID 与配置不变，释放资源后 Rust 下载重试成功。拆除挂载前停服务并复制回最新信任
状态，不能因为测试收尾而恢复旧 journal。

## 实测发现与修正

第一轮 TUF 验签成功，但目标下载返回 404：两个实现对嵌套目标的摘要前缀路径不同。
正式契约改为固定平面目标名称，仍校验产品/组件/渠道/平台等签名字段，没有改弱签名或增加
任意 URL 回退。初次单元检查也调整为基线 enrollment 创建 active/candidate 后才验证可执行路径。

检查库源码确认 datastore 写入本身不是应用需要的持久事务。Rust 包装层因此分离工作区与
不可变快照，在下一次网络请求前保存已验证 root 历史，并通过库本身重放链条。
不能在 root 换钥清理旧角色缓存尚未完成时，把新 root 与旧角色缓存提交成一个完成的刷新。
元数据下限先于目标下载持久化；程序回滚不回滚这些记录。

首次冷启动配置摘要变化是实验串扰：旧 Python 原型的 Agent 虽然停止，却仍启用了开机启动，
与新的固定 Agent 共用测试 node ID。已显式禁用旧原型探针、观察器、timer/path，再完整复跑。
没有以放宽配置比较来掩盖该失败，且没有操作现网凭证。

另外补上目录/可执行文件的精确所有者与权限检查、CLI 的私有 umask、未引用缓存的有界回收，
以及 pending 条件，避免空闲时每秒启动进程和散列对象。失败后的不确定激活状态要求人工恢复。

## 证据与上线限制

最终用例数、通过状态、源码与二进制摘要由 `rust-matrix-evidence.json` 记录；开始构建前和
结束验证后比对源码快照，源码中途变化的结果不能标成完整。非敏感日志/JSON 取回本地忽略
目录，客机磁盘、随机凭证、管理员密码和签名私钥不取回。

本轮本地证据位于 `output/ops/agent-update-rust-20261019/`，云端原始证据位于
`~/NodeLite/output/agent-update-prototype/`。矩阵、`build.json` 和本地审查逐项核对了
95 个源码输入，客机清单又核对了 12 个 unit 与二进制摘要。正常 updater 的 SHA256 为
`b58f78a7638ec27495e4811f6ae8fdd296a5394627a128cb93a8abb1d4b4cda6`；
故障注入版本为 `89149a80c3a738bb054d1afb96253860a6c60c11c12f2924f19bd77426e8a55a`。
正常程序不含实验客机标记路径；两个 musl Agent 均经 `readelf` 确认没有动态解释器。

收尾已停止并禁用实验服务、拆除测试挂载、移除故障注入 drop-in，客机恢复正常 updater。
随后停止 QEMU，核对 PID 文件消失、SSH 转发端口关闭，编译宿主没有安装任何
`nodelite-*`/`nlup-lab-*` 单元。实验磁盘和私有文件仍只保留在编译机私有输出目录，便于复查。
复现入口为 `experiments/agent-update-isolation/rust_matrix.py`，前置条件见其
[实验说明](../experiments/agent-update-isolation/README.md#actual-rust-updater-signed-download-through-activation)。

这已是连接完整流程的 Rust 实现，但不表示本轮发布或启用了生产自动更新。
生产签名仓库、离线 root 保管/轮换/吊销发布流程、首次迁移工具和独立安全审查仍需单独完成。
未验证真实外部代理、可信时钟恢复、硬件存储控制器故障、任意 syscall 崩溃或发行版/架构矩阵。
控制面和 Agent UID 失陷不应获得主机 root；updater UID 与发布签名基础设施仍属于可信计算基。
