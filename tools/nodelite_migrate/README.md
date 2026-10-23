# 显式的首次迁移

普通用户使用[根 README 的一条安装／升级命令](../../README.md#可选的签名-agent-升级)，无需
运行本目录的 Python 命令或管理签名。以下是发布维护者的底层接口与故障排查说明。

本地管理员使用的工具，将原手动安装迁到双用户 Rust 签名更新器。默认只做检查，修改必须显式
`--apply`。不接收网页/Server 的迁移指令，不赋予 updater sudo，不修改现网。操作密钥和仓库见
[签名说明](../nodelite_release/README.md)。

## 目录与代码职责

| 文件 | 职责 |
| --- | --- |
| `__main__.py`、`__init__.py` | Python 包入口，分发检查、apply、resume、recover 和 audit。 |
| `bundle.py` | 在特权写入前验证迁移包清单、独立指纹和本地策略。 |
| `common.py` | 固定安装路径、root/权限检查、受限 systemd 调用和停机/凭证持久化屏障。 |
| `transaction.py` | 建立账户与目录，持久化迁移状态，编排切换、试运行和恢复。 |
| `audit.py` | 迁移后检查固定单元、身份、安装摘要和近期健康证据，不读取 Token。 |
| `installer.py` | 校验安装器内置的登记信息，复用 apply/resume；调用固定更新单元并等待实际提交，不把回滚报为成功。 |
| `test_*.py` | 迁移包与准入边界回归；测试命令见[根 README](../../README.md#本地编译与测试)。 |

这些模块是实际迁移和开机恢复所需源码，不是实验夹具。节点只需 Python 标准库；文件安全
操作复用 `tools/nodelite_release/io.py`，不加载其签名库。迁移时安装的恢复副本由 root 保护。

## 支持边界

仅支持非容器 Linux、systemd、cgroup v2，默认启用且在线的 Agent 安装：

- 原程序 `/usr/local/bin/nodelite-agent`、原 unit 位于 `/etc/systemd/system/nodelite-agent.service`；
- 专属 `nodelite-agent` 用户和同名主组，配置 `/var/lib/nodelite-agent/agent.toml` 为 0600，目录 0700；
- 原 Agent 至少 v3.0.4（包含凭证持久化修复），无文件/ambient capabilities、流量控制或自定义 drop-in；
- `report_interval_secs` 为 1–10 秒；候选只读预检在停止原 Agent 前拒绝不兼容配置，不自动改值；
- root 保护程序、迁移源代码和所有祖先目录，专属账户没有额外组或共享 UID；
- 新包包含健康证据功能的已签名 Agent，**旧 v3.0.4 本身不具备该证据**。

容器、tc、CAP_NET_ADMIN、非默认路径/组/环境文件等拒绝迁移；不要自动放宽检查。Agent 不提供
`--version`，工具使用不联网的 `--sample-once` 身份字段检查版本，不输出采样或 Token。

## 准备与检查

可信发布机通过已验证仓库组包；普通节点只需要 Python 标准库及 Rust updater，不安装 TUF
Python 依赖。组包必须使用**正常构建** updater，禁止 `lab-faults`：

```sh
python -m tools.nodelite_release pack-enrollment --repository /srv/nodelite-repository --updater /build/nodelite-updater --units nodelite-updater/systemd --out /review/enrollment
```

返回 manifest 和 root 的公开 SHA256。管理员独立核对 root、updater、固定 systemd 单元和整包
摘要，通过可信渠道把源代码和包放进 root 保护的本地路径。manifest 只允许精确的 17 个文件，
不允许任意脚本、路径或密钥；所有字节先快照、校验，再做 root 写入。

下面从 root 所有、组/其他用户不可写的源码目录执行，摘要和地址均为占位符：

```sh
sudo python3 -m tools.nodelite_migrate --bundle /root/review/enrollment --bundle-sha256 BUNDLE_SHA256 --root-sha256 ROOT_SHA256 --metadata-url https://updates.example.invalid/stable/x86_64/metadata/ --targets-url https://updates.example.invalid/stable/x86_64/targets/
```

检查模式不更改 Agent 服务或配置。确认维护窗口和输出后，以相同参数追加 `--apply`。
已有 updater/migration 状态会拒绝新的 apply，不能用重复安装覆盖信任记录。正常启用后的
旧版 `install-agent.sh` 拒绝覆盖 signed updater 的 active 路径；新版安装器自动转到本地签名
更新器，保留原策略和信任记录，不用远端包替换它们。

## 事务与恢复

迁移顺序为 `prepared → stopping → switching → trial → committed`：

1. root 私有目录保留原 unit 和二进制；**不复制或备份配置/Token**。
2. 安装 root 管理的固定恢复程序和开机单元；账户/目录布局见 updater README。
3. 以 updater UID 验签，要求包中的 baseline 与仓库当前授权的摘要/序号完全一致。
4. 以 Agent UID、只读配置运行凭证预检；失败时旧 PID 继续运行。
5. 停止旧进程、确认空 cgroup，fsync 最新配置；先持久化恢复依赖 drop-in，再替换固定 unit，
   保证任意切换断电边界都有 fail-closed 恢复依赖；不并行运行两个 Agent。
6. 新 Agent 鉴权上报并持续推进健康证据后，启用固定 updater path/timer，提交迁移成功。

管理工具属于本地 root 管理边界，与日常非特权 updater 分离。它只在首次迁移时额外读取
`/proc/PID/exe` 核对实际程序；不为 updater 用户开放跨 UID 特权。

发生试运行失败或未提交的断电，root 恢复单元只恢复原 unit、启动原程序；**最新配置和 Token
永不回滚**。已接受的根链、元数据和 release 下限同样保留。恢复备份损坏或状态不一致时失败
关闭，要求人工排查；不能拿任意旧包启动。`committed` 后不再授权该恢复单元降级旧程序。
若签名最低回滚序号已高于 1，原手动程序必须先由管理员替换为同一当前授权摘要，否则拒绝
自动迁移，避免首次回退到已吊销旧程序。

| 新增 root 管理路径 | 用途 |
| --- | --- |
| `/var/lib/nodelite-updater-migration/`，0700 | 原 unit/程序、首次 receipt、状态与重试历史，文件 0600 |
| `/usr/local/lib/nodelite-operator/` | 审查后固定的恢复程序，仅 root 可写 |
| `/usr/local/lib/nodelite-enrollment-agent` | 本次经指纹核对的 baseline，只用于登记/迁移 |
| `nodelite-updater-migration-recover.service` | 启动前恢复未完成迁移，成功后保持 active(exited) |
| Agent 的 `10-enrollment-recovery.conf` | Requires/After 固定恢复单元，冷启动恢复失败则不启动 Agent |

手动恢复使用已安装的固定程序：

```sh
sudo python3 -I /usr/local/lib/nodelite-operator/recover.py --recover
```

原程序和配置已恢复在线、journal 为 `rolled_back` 后，修复网络或凭证问题，再用原参数追加
`--resume`。不能用 `--apply` 重新初始化。若仓库已前进，需要重新独立核对 enrollment 包，
用原 bootstrap root 版本组包；只允许更新 Agent baseline，不允许改变 root、updater、固定
单元、URL、渠道、架构或原 rollback floor。提高回滚下限需要另外人工处理。

resume 保留所有既有 TUF 状态；发现“信任处理已开始，但 journal 消失”时拒绝静默重置。
开机恢复服务必须已处于 active(exited)，否则先检查并启动该固定服务。
已经切换的正式 Agent 不允许再调用 `verify-enrollment` 改变 active 指针。

## 迁移后准入检查

迁移提交后，以及首次更新/重启后，从同一可信源码目录运行：

```sh
sudo python3 -m tools.nodelite_migrate --audit
```

该命令核对首次 receipt、root/updater 指纹、12 个固定单元、恢复依赖、账户/配置权限、启用
状态、updater 终态及当前实际进程的近期鉴权健康证据。不读取配置或 Token，不改变服务或
凭证；updater 的 status 可能创建锁、清理终态 pending 或标记坏 journal，不能把 audit 理解
为零本地状态写入。任何拒绝或非 ready 都暂停推广，保留证据排查，不能自动删除 journal。

本检查不是远程证明：主机 root、updater 身份或 Agent 本身已被攻破不在其保证范围内。

安装器的 `--enable-signed-updates` 从同一具体 Release 下载、校验 helper 和公开迁移归档，
使用 root 私有临时目录，拒绝归档路径穿越、链接、重复条目和超限文件；SHA256 校验成功后
才加载迁移代码。首次信任继承管理员选择的 HTTPS Release 来源，不能只拿未核验的面板脚本
当成独立信任证明。归档中的 root 指纹/固定 URL 由发布方绑定，后续更新只信任本地根和策略。
节点需要 OS Python >=3.9（只有标准库）；未安装时由 apt/dnf 自动准备，不能准备则明确失败，
不会偷偷跳过恢复能力。每日数据采集和签名更新核心仍是独立 Rust 程序。

本轮不提供批量自动迁移，不自动创建生产密钥，不自动重发节点凭证，也不以
删除 `/var/lib/nodelite-updater/private`、旧 journal 或信任根作为恢复方案。
