# Agent 更新故障注入验证（第四阶段）

日期：2026-10-19（Asia/Singapore）。承接[第三阶段](agent-update-release-network-validation.md)。

后续独立 Rust 更新器、签名下载到激活的连接见[第五阶段报告](agent-update-rust-validation.md)。

## 范围与结果

仅使用云端 `compiler` 的 `~/NodeLite` 和带专用标记的 KVM 客机。
没有操作现网节点，没有接网页、推送、发布或安装生产更新器。
没有复制现网凭证、数据库或 SSH 私钥；实验凭证只留在私有客机磁盘。

| 新增实验 | 结果 | 实际注入 |
| --- | --- | --- |
| HTTP 故障 | 7/7 | 连接重置、503、模拟代理 407、404、循环重定向、不返回响应头、持续慢速响应 |
| 持久信任缓存 | 6/6 | 下载失败后降级、部分刷新后的密钥吊销、丢失/损坏状态、损坏元数据/产物 |
| 缓存断电 | 5/5 | 准备、状态 rename 后但目录 fsync 前、元数据已提交、产物已持久化、产物已提交 |
| 客机时钟 | 2/2 | 倒退两小时、快进两天，恢复时钟后重试 |
| 真实文件系统耗尽 | 5/5 | 缓存满盘、inode 耗尽、下载中空间不足、真实 Token 满盘/inode 耗尽 |

共新增 25 项故障注入用例；另复跑原有 16 项参考 TUF HTTP 测试。
Python 离线回归共 35 项，通过。云端 `cargo test --workspace --locked` 共 646 项通过、
3 项忽略；`cargo clippy --all-targets --locked -- -D warnings`、`cargo fmt --all -- --check` 通过。
后端检查仍使用 `NODELITE_SKIP_WEB_BUILD=1`，没有声称本轮前端或 GitHub CI 通过。

## 缓存不再与产物一起前进

原先的 HTTP 实验仅在下载成功后保存应用版本下限。这不足以覆盖“已接受新版清单，
但新程序下载失败”的窗口；因此另加 `DurableClient`，不把旧实验直接当作生产实现。

新实验在同一事务锁内，复制上一代已提交的参考 TUF 缓存，在新目录中验证并刷新：

1. 先 fsync 元数据文件和目录，再通过原子状态文件提交新 generation、release sequence/hash。
2. 然后下载产物，校验后原子持久化到独立对象目录。
3. 最后单独提交产物指针；此前始终保留旧产物。

这样既保留旧程序字节，又不会把已经接受的版本下限降回旧值。
实测新清单已提交但下载失败后，即使镜像给出更新 metadata version、合法签名的旧 release，
仍因应用版本下限被拒绝；恢复空间/镜像后可下载新版。

参考库可能先接受 root 轮换，再因 targets 签名失败退出。异常路径也保存已验证的缓存前缀，
不丢掉已学到的吊销。实测移除镜像的新 root 文件后，再给出被撤销密钥签署的 targets，
仍然拒绝，随后由新密钥签署的版本可以下载。

状态文件绑定初始 root 摘要和元数据清单；状态缺失、内容损坏、产物损坏、非法链接等
都要求显式恢复，不能自动清空缓存并忘记下限或吊销。初始化是独立、拒绝非空目录的动作。
整个事务使用非阻塞 flock，避免两个下载器同时修改状态。没有把 Agent 配置或 Token 放入缓存。

## 真正冷启动，而不是重跑 Python 函数

五个位置均直接 SIGKILL QEMU，随后从原 qcow2 磁盘启动，并核对 boot ID 改变。
checkpoint 通过客机 `/run` 的 tmpfs 通知，避免通知文件的 fsync 意外刷盘。

在元数据持久提交之后的三次断电中，版本下限保留为 2，旧 release 被拒绝。
在产物指针提交前仍保留旧产物；提交后保留新产物；所有场景可重试至正式 v3.0.4 的正确摘要。
状态 rename 后但目录尚未 fsync 的位置，本轮恢复为完整旧状态，而非半写 JSON。
这一位置尚未完成持久提交，允许旧或新完整状态，不把尚未 fsync 的状态当作已确认的下限。

首次试跑暴露的是实验基线错误：刚同步的测试脚本和单元未刷盘，断电后脚本变成空文件。
已在启动故障事务前显式同步测试源码、镜像和固定单元，完整复跑；失败试跑不计入结果。
测试缓存重跑时归档旧目录，不覆盖此前现场。

客机下载器使用无 sudo 权限的独立 `nlup-updater` UID，而不是具有测试 sudo 权限的 SSH 用户。
执行代码由 root 放在只读目录；结果记录非零 UID 和零 effective capabilities。
固定镜像仅监听 loopback，签名私钥只在编译机发布实验进程内存中生成，不传入客机。

## 时钟、网络与实际 ENOSPC

本地已接受时间下限拒绝两小时倒退；快进两天则由参考 TUF 到期检查拒绝。
两种失败均未抬高时间下限、未替换旧产物。实验控制器按原时钟加 monotonic 经过时间
恢复客机时钟，并核对与宿主时间接近，再重试成功。没有改变宿主时钟。
这不是可信时间修复机制，更不能防御恶意 root 或整机快照回滚。

真实网络实验使用 loopback socket，而非仅 mock 异常。连接读超时为一秒，元数据刷新与
目标下载各有五秒总时限；持续发一个字节的连接仍在约五秒中止。POSIX alarm 仅适用于此主线程实验，
不是未来 Rust 异步取消机制。407 是模拟返回码，没有冒充真实外部代理/CONNECT 验证。

存储实验在客机内创建独立 64 MiB ext4 普通文件镜像，以 nosuid/nodev/noexec 挂载。
分别真正写至可用块为零、创建文件至可用 inode 为零，捕获 errno 28；没有写满客机根盘
或编译机磁盘。另一次仅留下 512 KiB，使元数据可以提交、正式 Agent 产物无法完整落盘，
验证版本下限已为 2 而旧产物仍保留。释放填充文件后重试全部成功。

Token 测试运行官方 v3.0.4 Server CLI 和 Agent，将其配置目录绑定到独立测试文件系统。
满盘及 inode 耗尽时，在线轮换不会收到成功 ACK，原配置字节和 Agent PID 保持不变，
实际鉴权上报继续健康；释放资源后再次轮换成功，磁盘配置改变，官方
`--check-credentials` 通过。卸载前停 Agent，把最新配置写回底层目录，避免收尾恢复旧 Token。
卸载前检查 loop 的 backing file；全部挂载仅在客机中进行。

## 复现、证据与边界

命令和依赖见[实验说明](../experiments/agent-update-isolation/README.md)。非敏感日志和 JSON
归档于本地忽略目录 `output/ops/agent-update-resilience-20261019/`，不取回客机磁盘、
测试签名私钥、管理员密码或 Token 配置。JSON 要求 `complete: true`、精确用例数和源码摘要。

| 证据文件 | SHA256 |
| --- | --- |
| `network-resilience-evidence.json` | `82c48b14fb10d162b5775131f73ae813ccc665fc479bda80307faddd822f8c3c` |
| `durable-network-evidence.json` | `778482977f121fd9aead6161ace2c1bcac594d7c37bb3401827752ce6d3b3efb` |
| `cache-powercut-evidence.json` | `f607804c7bdbae79b89487a922d1fb883a72661e7d28e34b4613e34eb7a80db4` |
| `storage-resilience-evidence.json` | `cec82fa6b161e72eecba79b153125273ed20f2194e84ae124990085ec4359f89` |
| `tuf-network-evidence.json` | `f12205c92731b96f446cf52b6de89553c00cf551ade5f6a4efce027038b1ca06` |

已核对客机实际安装的源码摘要与本地一致。18 个实验单元验证成功，只有系统自带
`snapd.service` 的旧版本 systemd 警告；没有修改该单元。收尾确认无活动实验单元、
无残余测试挂载，再关闭 QEMU；宿主无实验单元，22239 转发端口已关闭。
私有镜像和已归档缓存留在云端项目 output，便于复查。

本轮仍不代表可上线：下载缓存与旧的激活协调器尚未连成生产 Rust 流程；还缺少正式
健康 IPC、opt-in 安装、生产密钥保管与吊销发布流程、管理认证审计和独立安全审查。
单一 Ubuntu/systemd/KVM 的选定故障点，不等于任意 syscall、存储控制器、外部网络和多平台
的穷尽验证。旧 generation/对象的安全清理和长期空间配额也尚非生产实现。
