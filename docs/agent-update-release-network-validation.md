# Agent 正式产物、真实断电与签名下载验证（第三阶段）

日期：2026-10-19（Asia/Singapore）。

后续缓存断电、时钟偏移、网络限时和真实满盘/inode 测试见
[第四阶段报告](agent-update-resilience-validation.md)；下列数字保留为本阶段的独立记录。

## 结论与范围

在云端编译机 `compiler` 的 `~/NodeLite` 和专用 KVM 客机中完成验证。
没有操作 HK、Oracle、Korea 等现网节点，没有接网页、推送或发布。
没有把现网配置、token、数据库或 SSH 私钥放进实验环境。

| 验证组 | 最终结果 | 实际覆盖 |
| --- | --- | --- |
| 正式 Agent 兼容 | 8/8 | 官方 v3.0.3 → v3.0.4 ELF、轮换后程序回滚、三处协调器中断 |
| 完整 Server 与断电 | 9/9 | 两项正式 Server 升级/重启，七处真实凭证窗口 SIGKILL QEMU 后冷启动 |
| 签名 HTTP 下载 | 16/16 | 参考 TUF 实现、真实 loopback HTTP、坏包/重放/换钥/吊销 |
| Python 离线回归 | 22/22 | 原有 17 项、新增 4 项观察器边界及 1 项导出回归 |

全量 `cargo test --workspace --locked`、`cargo clippy --all-targets --locked -- -D warnings`、
`cargo fmt --all -- --check` 通过。后端构建仍显式设置 `NODELITE_SKIP_WEB_BUILD=1`，
未运行或声称通过本次前端构建、浏览器测试或 GitHub CI。
客机 16 个实验单元的 `systemd-analyze verify` 返回成功；系统自带 `snapd.service`
有 systemd 249 忽略 `RestartMode` 的警告，未修改该系统单元。

## 正式产物与兼容边界

直接下载 [v3.0.3](https://github.com/XiNian-dada/NodeLite/releases/tag/v3.0.3) 和
[v3.0.4](https://github.com/XiNian-dada/NodeLite/releases/tag/v3.0.4) 的 x86_64 Linux musl
Server/Agent，与各 tag 的 `SHA256SUMS.txt` 对照。证据保留下载 URL、摘要和大小。
这只是 HTTPS/同源校验和引导，不是独立发布者签名。旧 Agent 没有 `--version` 参数，
因此通过实际鉴权后的版本字段与 `/proc/<pid>/exe` 摘要确认身份，不能只看文件名。

v3.0.3 Agent 不支持持久化 ACK，v3.0.4 Server 会拒绝对其执行在线 token 轮换。
实测该拒绝不修改磁盘配置、不替换健康进程；升级到 v3.0.4 后再轮换成功。
另一路径在 v3.0.4 保存新 token 后故意卡住新版，回滚到 v3.0.3，仍能使用最新配置鉴权。

这一组 Server 是生产 WS/registry/refresh 处理链的测试脚手架 `TestServer`；完整正式 Server CLI 的
升级、后台任务、注册表冷加载和重启由下一组单独验证，不能混为同一个证明。

## 完整 Server 与真实凭证一起断电

客机运行官方 v3.0.4 Agent、官方 Server CLI，以及从当前源码构建、标为 `lab-next`
的候选 Agent。候选不是未来正式版本；这一组不是另一个正式版本配对。
Server 使用独立、零 capability 的 `nlup-server` 用户和持久注册表/数据库。
所有 HTTP/WS 只监听客机 loopback，GeoIP 外发关闭。

除 Server v3.0.3 → v3.0.4 迁移和 v3.0.4 轮换后的重启外，逐一在以下位置直接杀掉
QEMU，Server 与 Agent 同时失电，再从原磁盘冷启动：

1. token 轮换响应尚未送达：旧磁盘凭证可恢复。
2. 新 token 已落盘、ACK 被截留：Server 从持久候选识别新凭证。
3. PREPARED 已落盘。
4. STOPPING 已落盘。
5. TRIAL 已落盘、active 尚未切换。
6. active 已切换、候选尚未启动。
7. 候选实际运行并再次轮换 token、尚未提交升级。

每项验证新的 boot ID、预期终态、基线可执行文件身份、Server 新鲜采样、配置字节不变，
并由官方 v3.0.4 `--check-credentials` 再验证。额外观察三秒，避免遗漏迟到的服务动作。
升级回退只改变程序，未恢复旧配置或 token。最后一项确实回到正式 v3.0.4 程序，
仍使用候选轮换的新 token。

## 实测发现的观察器问题

最初 TRIAL 冷启动停在 `rollback_intent`，虽然 Agent 已在线，协调器仍报告恢复超时。
现场确认心跳是 `0600`：测试观察器先 chown 给 Agent，再 chmod，但自身没有
`CAP_FOWNER`，chmod 失败；协调器读不到文件，正确地拒绝宣告恢复。

修复采用观察器的主组 `nlup-read`，在原子 rename 前设置 `0640`，不再依赖发布后的
chown/chmod，也不增加 Agent/updater 权限。观察器仅保留检查跨 UID 进程所需的
`CAP_SYS_PTRACE` 和写测试目录所需的 `CAP_DAC_OVERRIDE`。单独重现和完整矩阵复跑均通过。
另补上 Python 3.10 无法直接解析 chrono 九位小数时间戳的回归。

观察器是**可信测试仪器**，不是生产健康 IPC 或可交付的 root 常驻组件。

收尾还发现证据输出目标名与安装 worker 的循环变量重名：测试完成后，JSON 被误写到
output 内的 `lab_io.py`，正式结果文件没有生成。已改用独立文件名变量，并增加不调用
SSH/编译的回归，检查目标路径、二进制清单和同名文件不被覆盖；随后复跑真实测试与导出。

## 签名与下载链路

独立实验使用 [python-tuf 的 Updater](https://theupdateframework.readthedocs.io/en/latest/api/tuf.ngclient.updater.html)
及 Metadata API，通过真实 HTTP 下载上述官方 Agent 字节。密钥只在单个实验进程内生成；
root 需要 2-of-3，targets 需要 2-of-2，包含 root/timestamp/snapshot/targets 信任链。
应用层另校验产品、组件、渠道、平台、大小和已接受的 release sequence/hash。

测试包括内容篡改、截断、超长文件/元数据、签名不足、过期、scope 错误、旧 timestamp
重放、合法签名的应用版本降级、同一 release 对应不同字节、元数据混搭、root 正常轮换、
旧/新 root 阈值不足和被吊销 targets 密钥。每例确认经过实际 HTTP，而非直接调用校验器。
截断响应在此依赖组合中抛出 urllib3 `ProtocolError`；集成方也必须把传输错误视为失败，
不能仅捕获 TUF 校验错误。所有拒绝路径均未激活下载产物。

参考库只进入云端项目 output 下的独立 Python 环境，没有增加 Rust/正式安装器依赖。
现有实验协调器仍接收原来的本地签名包，**尚未把 TUF 下载与程序激活连成生产流程**。

## 证据与复现

复现命令、环境和安全限制见 [实验说明](../experiments/agent-update-isolation/README.md)。
非敏感证据位于本地忽略目录 `output/ops/agent-update-release-20261019/`。
冷启动证据要求 `complete: true`、恰好九项、安装代码/unit/二进制摘要与服务停止记录；
签名实验记录依赖版本及源文件摘要。已核对安装源码与本地源码一致。

| 文件 | SHA256 |
| --- | --- |
| `official-real-evidence.json` | `7faedba77c3954caa13a0ec9504be3efacf7d82952f73d8e670fa00885bf0fd3` |
| `official-cold-evidence.json` | `cac8f1c266b50d982676d374ef971f8d7b282a94e6a34f1f8479a21e978066d5` |
| `tuf-network-evidence.json` | `5dbb12069d49ac27156584990b9c44b42d50c34daf4d590e2c89545d62a72169` |
| `python-official.log` | `6e990df5eb58073eecbc66f0884fbbb3087cb6edd8d04bd0c4cf5a40b96df1ce` |

最终确认客机没有活动实验单元，再关闭 QEMU；宿主没有 `nlup-lab-*` 单元，
22239 转发端口没有监听。内核为 `5.15.0-198-generic`，systemd 为 249。
私有客机磁盘、随机凭证和实验密钥仍只保留在云端项目 output，未取回这些私有文件。

## 尚不代表可上线

- 仍需生产 Rust 更新器、正式健康/能力协议、opt-in 安装与管理认证/审计。
- TUF 实验密钥不来自独立签名保管人，尚无生产签名、离线 root 操作与吊销发布流程。
- 尚未验证 TUF 缓存本身的断电持久性、可信时间修复、真实外部镜像故障和代理异常。
- 单一 Ubuntu/systemd/KVM、loopback 网络，不代表发行版/架构/网络延迟和权限策略矩阵。
- 未覆盖每个 syscall 内部崩溃、真实满盘/inode 耗尽或存储控制器故障；仍需独立安全审查。

这些结果补上了第二阶段的正式版本、完整 Server 冷启动和真实 token 断电缺口，
并证明所选恢复路径可工作；不等于可以在现网打开自动升级。
