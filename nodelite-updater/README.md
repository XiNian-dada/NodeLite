# NodeLite Rust Agent 更新器

独立的 Linux/systemd CLI，连接 TUF 签名下载、只读预检、停止屏障、试运行与单次回滚。
默认关闭，不接收 Server 下发的 URL、命令、服务名或安装路径，不包含网页接口。
不会自动迁移任何节点；原手动安装流程仅新增防止覆盖已启用签名更新器的检查。

当前已在隔离 KVM 客机验证。[签名仓库与密钥工具](../tools/nodelite_release/README.md)及
[显式首次迁移工具](../tools/nodelite_migrate/README.md)已补齐，但尚未部署生产签名源、
建立真实多方密钥保管或启用官方发布流程。不要把 `experiments/` 的 root 测试脚本安装到现网。

## 边界与目录

首版仅支持非容器的 Linux/systemd、cgroup v2、无 capabilities 的 Agent。
开启流量控制、`CAP_NET_ADMIN`、非 systemd、macOS 等环境仍使用原有终端升级方式。
更新器拒绝以 root 或错误用户/主组运行。初次启用必须由管理员登记可信基线，不能假定
旧的、未签名的安装程序已经获得新的发布授权。

| 路径 | 所有者与权限 | 用途 |
| --- | --- | --- |
| `/usr/local/bin/nodelite-updater` | root，0555 | 独立更新器，不自我替换 |
| `/etc/nodelite-updater/` | root:root，0755；文件 0644 | 本地策略与离线引导的 `root.json` |
| `/var/lib/nodelite-updater/` | root:root，0755 | 不可被普通用户替换的父目录 |
| `code/`、`code/objects/` | nodelite-updater:nodelite-read，0750 | 按 SHA256 命名的不可写可执行对象，0550 |
| `private/` | nodelite-updater:nodelite-updater，0700 | journal、互斥锁、信任历史与缓存；文件 0600 |
| `bridge/` | root:nodelite-updater，0770 | 仅三个固定动作标记：preflight、stop、restart |
| `/var/lib/nodelite-agent/` | nodelite-agent，0700；配置 0600 | 原有可写 Agent 配置，更新器不可直接读取 |
| `/run/nodelite-agent/` | nodelite-agent:nodelite-read，0750 | 0640 的本地非敏感健康证据 |

Agent 与 updater 是不同用户；二者都不能修改 root 的策略和单元。updater 是可信计算基：
其完全失陷可通过替换 Agent 代码间接读取 Token，但固定 root 动作和空 capabilities
不授予其主机 root 权限。不要在保持 updater 代码写权限时，另给 Agent unit 增加 capabilities。

## 签名下载与状态

使用固定版本 `tough 0.24.0` 校验 TUF root、timestamp、snapshot 和 targets。
初始 root 必须通过独立可信渠道交付，不能仅从受控面板下载一份公钥就视为可信。
更新器不持有发布私钥，也不执行下载来源提供的脚本。

目标名称固定为 `nodelite-agent-<channel>-<target>`，例如
`nodelite-agent-stable-x86_64-unknown-linux-musl`。首版使用平面名称，避免不同客户端对
带目录目标添加一致性快照摘要前缀的互操作差异。targets 中的 `custom` 必须恰好包含：

```json
{
  "product": "nodelite",
  "component": "agent",
  "channel": "stable",
  "target": "x86_64-unknown-linux-musl",
  "release_sequence": 2,
  "min_rollback_sequence": 1,
  "version": "3.0.5"
}
```

版本字符串仅为发布标签，不是命令。防降级依赖单调的 release sequence、摘要与持久信任状态，
不是字符串版本比较。渠道只允许 stable/testing，平台只允许本机对应的 Linux musl 平台。
策略示例见 [policy.example.toml](policy.example.toml)，其中 `enabled = false`，占位地址不可用。

网络只访问 root 策略指定的 metadata/targets 前缀，HTTPS 验证保持开启；HTTP 仅允许字面 loopback
用于本机仓库测试。不跟随重定向，不接受 URL 用户信息、查询参数和 fragment。
单请求总时限 10 秒、整轮传输预算 45 秒，目标不超过 32 MiB，元数据另有更小上限。
根元数据的 503/超时/超长响应也会使本轮失败，不能被当成“下一份 root 不存在”。

TUF 库写入的工作区与已提交缓存分离。每个已验证 root 在下一次联网之前持久化到独立历史，
后续通过同一个 TUF 库重放验证，镜像不能通过隐藏新 root 恢复已吊销密钥。
root 轮换期间不提前把旧角色缓存与新 root 混为一个完成的更新，保留库自身的换钥清理流程。
元数据和应用版本下限在目标下载前提交，下载失败仍保留旧程序和新的信任下限。

锁覆盖整个事务。缺失/损坏状态、摘要不符、非法 inode/链接或不安全目录权限要求人工恢复，
不会自动清空缓存、重新登记或回到初始公钥。程序回滚不回滚配置、Token、元数据下限或信任历史。
已确认的本地时间下限拒绝明显倒退，但不提供可信时间修复、TPM 防回滚或快照回滚防护。

缓存回收只发生在持锁、无待激活事务的下载开始时，仅删除未引用的平面缓存与对象。
当前缓存、当前程序、待升级摘要以及最后一项事务的 from/to 始终保留；root 历史与 journal 不清空。
拒绝链接、异常权限和未知条目。工作代次最多 128，root 历史最多 128；达到防御上限时拒绝继续，
需管理员保留信任记录进行维护，而不是删除整个 private 目录。

## 激活与恢复

固定的 root bridge 只运行系统自带的 `rm` 和固定参数 `systemctl`，不读取可写 shell/环境文件，
不运行 updater 可写程序，不创建任意临时单元，也不安装 Polkit 或免密 sudo 规则。

`PREPARED → STOPPING → TRIAL → COMMITTED`；失败进入 `ROLLBACK_INTENT → ROLLED_BACK`，
不一致时转人工。journal 先于切换持久化；active 只是可重建指针。
停止必须同时满足固定 stop 单元的新完成记录、动作标记已消费、MainPID=0、inactive、空 cgroup。
候选只在旧进程完整退出之后启动，避免两个正式会话竞争和跨进程 Token 写入冲突。

预检以 Agent 用户、配置只读的单元执行现有 `--check-credentials`，不会挤掉在线会话。
Agent 启动加载配置时，既有配置 I/O 会先同步凭证和目录再握手；轮换依旧在持久化后才 ACK。
正式试运行的健康证据仅在已鉴权会话成功发送采样后写入，包含 PID、systemd invocation、
实际执行映像摘要、采样计数和采集时间，不包含 Token。
更新器同时核对固定 unit、active 摘要、PID/invocation、时钟新鲜度和持续推进的采集时间。
这是 Agent 自报证据与固定单元身份的结合，不伪称拥有跨 UID 的特权进程观察器。

失败目标被记住，不自动再次试用；已提交事务不再授权自动降级。回滚允许有界的幂等重做，
不承诺 systemctl 物理调用恰好一次。旧基线低于本地/签名 hard floor 时拒绝可逆升级，要求人工处理。
开机恢复先清除旧动作标记，再依据 journal 恢复；旧 boot 的单调时钟不能继续使用。
空闲时 reconcile 单元通过 pending 条件跳过，不每秒启动 Rust 进程或散列程序。

默认健康稳定窗口 10 秒、试运行期限 60 秒。建议 Agent 上报间隔不超过 10 秒；过慢的上报无法
满足健康推进窗口，会安全回滚，不会跳过认证或健康检查。

## 构建与受控启用

在编译服务器的项目目录执行：

```sh
cargo build -p nodelite-updater --release --locked
cargo test -p nodelite-updater --all-features --locked
```

正常二进制没有故障注入能力。`lab-faults` 仅供带专用标记的隔离客机使用，必须单独构建，
不得作为发布二进制。验证脚本会记录两者不同的摘要。

首次启用优先使用上述显式迁移工具：审查完整包和独立 root 指纹，先检查再 `--apply`。
工具支持默认手动安装的 v3.0.4，先验签、只读预检，再切换到支持健康证据的签名 Agent；
迁移阶段另有 root 管理的冷启动恢复，提交后不授权自动降级原程序。
旧版 v3.0.4 本身没有本地健康证据，不能直接当作日常 updater 的健康基线。

以下是完成迁移后由本地管理员调用的日常检查，不是给更新用户授予 sudo 权限：

```sh
sudo -u nodelite-updater -g nodelite-read /usr/local/bin/nodelite-updater status
sudo -u nodelite-updater -g nodelite-read /usr/local/bin/nodelite-updater download
sudo systemctl start nodelite-updater-apply.service
```

初始化拒绝已有状态；下载命令不停止服务；apply 仅请求最新的本地策略授权目标。
`verify-enrollment` 仅供迁移时核对仓库当前授权基线，固定旧 unit 的身份检查阻止已迁移节点
再次调用它改变 active 指针。失败重试保留根链和版本下限，不重新 initialize。
本次没有自动启用安装器或网页按钮，实际隔离验证记录见
[Rust 更新器验证报告](../docs/agent-update-rust-validation.md)和
[签名与迁移验证报告](../docs/agent-update-signing-migration-validation.md)。
