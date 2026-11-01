# NodeLite Rust Agent 更新器

独立的 Linux/systemd CLI，连接 TUF 签名下载、只读预检、停止屏障、试运行与单次回滚。
默认关闭，不接收 Server 下发的 URL、命令、服务名或安装路径。
不会默认迁移节点；安装器可通过 `--enable-signed-updates` 显式开启，自动校验和迁移。
之后同一手动升级入口调用本机固定策略，不覆盖签名更新器，也不会无人值守拉取新版本。
额外使用 `--enable-web-updates` 才允许网页触发同一本地流程；仅开启签名模式不会隐式授权远程触发。

当前已在隔离 KVM 客机验证。[签名仓库与密钥工具](../tools/nodelite_release/README.md)及
[显式首次迁移工具](../tools/nodelite_migrate/README.md)已补齐，但尚未部署生产签名源、
建立真实多方密钥保管或发布正式迁移包。发布工作流已包含正常 updater 和安装 helper；签名
包必须由可信发布机审批后上传。运行时核心为 Rust 二进制，Python 标准库由安装器管理，
用于本地迁移／开机恢复／等待健康确认，不属于日常采集链路。

## 源码目录

- `src/`：Rust CLI。`policy.rs`/`fs.rs` 约束信任与文件访问，`download.rs`/`transport.rs`/
  `trust.rs` 负责签名下载与信任缓存，`state.rs`/`activate.rs`/`system.rs` 负责状态和激活恢复；
  `remote.rs` 消费有时限的本地触发并发布不含凭证的状态；测试与实现放在同一模块内。
- `systemd/`：14 个固定 Agent、预检、更新及桥接单元，不能用来自面板的服务名或命令替换。
- `policy.example.toml`：不含凭证、默认关闭的节点本地策略模板。

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
| `requests/` | root:nodelite-read，0770 | Agent 只能原子提交固定 `request.json`，更新器消费后再次校验 |
| `public/` | nodelite-updater:nodelite-read，0750；文件 0640 | Agent 只读的升级进度和结果，不含 Token 或配置 |
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

时钟曾错误前跳后，下载会因时间下限被拒绝。先校正并确认系统时钟，再由本地管理员显式重置：

```bash
sudo -u nodelite-updater -g nodelite-read /usr/local/bin/nodelite-updater reset-time-floor --confirm-clock-correct
```

该命令持事务锁运行，只把 journal 的 `time_high` 降到当前时间并输出新旧值，不改动已见序列与摘要、
hard floor、已安装版本或任何签名校验。缺少确认参数、事务未结束或存在 `recovery-required.json`
时拒绝执行；下限未超前时钟时不写入并以 0 退出。tough 的 `latest_known_time.json` 不进入已提交缓存，
无需另行处理。命令信任当前时钟，时钟未校正就执行会削弱时间下限，因此不要写入定时任务或脚本。

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
空闲时 reconcile 单元通过 pending 条件跳过，不每秒启动 Rust 进程或散列程序；
存在 `recovery-required.json` 时同样跳过，恢复现场保留给运维处理。
策略 `enabled = false` 只暂停新版本，未完成的事务仍按 journal 恢复或回滚；
仅当 `active` 已是已安装对象（人工恢复时也可为事务中未被吊销的 `from`）时，
开机恢复单元才成功退出、Agent 照常启动。journal 或对象校验失败、`active` 指向意外对象、
guard 或路径安全检查失败时，开机恢复单元仍会失败并阻止 Agent 启动（失败即关闭）。

默认健康稳定窗口 10 秒、试运行期限 60 秒。签名模式要求 Agent 上报间隔为 1–10 秒；
首次迁移和后续升级的只读预检会在停止旧 Agent 前拒绝不兼容配置。健康证据的 15 秒新鲜度
上限不变；需要更长上报间隔时继续使用原手动模式，不自动修改配置。

## 构建与受控启用

在编译服务器的项目目录执行：

```sh
cargo build -p nodelite-updater --release --locked
cargo test -p nodelite-updater --all-features --locked
```

正常二进制没有故障注入能力。`lab-faults` 仅供带专用标记的隔离客机使用，必须单独构建，
不得作为发布二进制，组包工具也会拒绝它。

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
网页触发不直接执行安装器；日常升级仍只使用 Rust 更新器和固定 systemd 单元。

## 网页触发与结果

本地 root 策略中的 `allow_remote_updates` 默认为 false。首次使用新版安装器的
`--enable-web-updates` 执行完整迁移和本地审计后，才开启该策略及 `nodelite-updater-remote.path`。
旧版实验安装不能通过网页升级更新器或固定单元，须先由本地管理员更新受信运行时。

`nodelite-proto/src/agent_update.rs` 定义严格的请求和状态类型。Server 与新版 Agent 双向协商
支持后才发送新消息；旧 Agent 保持正常上报。Server 的
`GET/POST /api/settings/agents/{node_id}/update` 使用现有认证、敏感操作短效确认和请求体上限。
`nodelite-server/src/agent_updates.rs` 只保存有容量上限、绑定当前会话的任务观察，不是授权源。

请求仅含随机任务 ID、签发及到期时间，有效期不超过 60 秒。Agent 检查本地策略和环境，
将请求写入固定目录；root 所有的 path 单元启动固定的非 root 更新服务。更新器重新检查
本地策略、请求期限、持久化的防重放/五分钟冷却及事务锁，再从本地固定签名源获取目标。
没有 Polkit、免密 sudo、可变服务名或远程脚本执行入口。

任务 ID 随 journal 持久化，状态经 `public/status.json`、Agent 已认证 WebSocket 和受保护 API
回传。新版停止旧进程后试运行，只有本地原有健康检查确认才返回 committed；rollback_intent
仍显示「正在回滚」。下载中断在恢复时标为失败，不在开机后无人值守继续下载；更新、回滚
和重连均不回退 Token。Server 重启后由 Agent 重新上报本地结果；未确认的请求显示 unknown，
不会把排队、断线或请求超时视为成功。

关闭网页触发时，将 root 策略中的 `allow_remote_updates` 改回 false，并禁用
`nodelite-updater-remote.path`；不要关闭恢复单元或删除 journal。已经开始的事务仍须完成
健康确认或安全恢复。面板自身更新与 Agent 更新是两条独立流程。

## 受控部署检查

1. 独立核对源码、正常 updater、14 个固定单元、整包及 root 指纹，禁止故障构建进入生产。
2. root/targets 与在线角色使用不同密钥，备份与口令分开保管。多把密钥由同一人持有不等于
   独立多方审批；私钥与面板同机时，即使加密保存也不能抵抗已失陷 root 对签名过程的窃取。
3. 按渠道/架构建立只读 HTTPS 仓库，HTTP 用户不能写，私钥、工作区和归档不在 web root，
   所有祖先目录均受保护。源地址只由节点本地 root 策略指定。
4. timestamp 至少每六小时续签，其他角色在到期前审批；将发布工具的 `check` 非零退出
   接入管理员告警并演练接收。配置时间同步、容量监控和公开历史保留策略。
5. 先选一台符合条件的节点，检查后显式迁移，再运行 `sudo python3 -m tools.nodelite_migrate --audit`。
   确认鉴权上报、download 不重启服务、已签名后续版本激活和重启后再次 audit 均正常；
   至少观察一次在线元数据续签后再扩大范围。

出现非 ready、manual_recovery、签名/摘要不符、异常会话、凭证落盘失败、到期或容量告警时
停止推广。保留当前 Token 和信任状态，不删除 journal/缓存/current，不重跑 initialize 或
降低回滚下限。未提交迁移的恢复方式见[迁移说明](../tools/nodelite_migrate/README.md)。

隔离运行验证覆盖 x86_64 / Ubuntu 22.04 / systemd 249 / cgroup v2；不能据此视为其他发行版
或架构的运行验收。健康证据不是主机未失陷的证明。
