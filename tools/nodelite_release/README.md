# 离线签名与静态发布仓库

本工具供发布管理员使用，不在 Agent、Server 或网页内运行。尚未配置生产密钥、生产更新源
或官方发布自动化；实验密钥不得转为生产密钥。首次迁移见
[迁移说明](../nodelite_migrate/README.md)。

## 目录与代码职责

| 文件 | 职责 |
| --- | --- |
| `__main__.py`、`__init__.py` | Python 包入口，通过 `python -m tools.nodelite_release` 调用。 |
| `keys.py` | 生成加密 Ed25519 私钥，解锁并生成绑定载荷的独立审批签名。 |
| `metadata.py` | 生成和校验 root、targets、snapshot、timestamp，合并阈值签名。 |
| `repository.py` | 验证完整发布链，在锁内发布不可变代次并切换 current。 |
| `bundle.py` | 将公开信任锚、Agent、updater 和固定单元组装为首次迁移包。 |
| `maintenance.py` | 独立指纹检查、到期告警和旧代次的可恢复归档。 |
| `io.py` | 有界文件读取、inode/权限检查和持久化写入；迁移工具也复用它。 |
| `test_*.py`、`requirements.txt` | 安全回归测试及锁定的发布工具依赖；测试命令见[开发文档](../../docs/development.md#构建与测试)。 |

此目录只放程序、测试和说明。私钥、审批工作区、公开仓库及归档均放在仓库外的受保护路径，
不要将真实数据添加到此目录。节点日常运行 Rust updater，不需要安装这里的 Python 依赖。

## 密钥分工

| TUF 角色 | 最低阈值 | 建议保管 | 默认有效期 |
| --- | --- | --- | --- |
| root | 2（推荐 2-of-3） | 三位独立保管人、离线 | 365 天 |
| targets | 2（推荐 2-of-2） | 两位独立发布审批人、离线 | 90 天 |
| snapshot | 1 | 与 timestamp 分离的受限签名环境 | 7 天 |
| timestamp | 1 | 可定期唤醒的受限在线签名环境 | 12 小时 |

角色之间禁止复用密钥。工具使用 python-tuf 生成标准 TUF metadata，使用 Ed25519；没有另造
签名格式。两份 detached approval 必须来自不同 key ID，且绑定同一角色的规范化签名载荷摘要。
输入 JSON 的缩进不是签名对象；签名前必须审查解析后的内容和 approval 中的 payload SHA256。

私钥是加密 PKCS8，父目录 0700、文件 0600，不覆盖已有文件。口令至少 12 字节，交互生成时
二次确认；非交互只允许显式传入已打开的 `--password-fd`。不要将口令放在命令参数、环境变量、
脚本、Git、CI 日志或公开仓库中。本实现不承诺 Python 内存清零、HSM 或抗已失陷签名机。

每位保管人应独立备份自己的加密私钥，口令另存；在隔离环境演练解密和审批恢复。不要把满足
整个阈值的生产密钥放在同一 CI job。实验中一个进程管理多把一次性密钥，仅证明协议和程序
互操作，不证明生产的多方保管成立。

## 安装与初始 root

仅在可信发布机、仓库根目录运行：

```sh
python3 -m venv /trusted/operator-venv
/trusted/operator-venv/bin/pip install --only-binary=:all: -r tools/nodelite_release/requirements.txt
```

以下用 `python` 代指该 venv 的 Python。示例路径为占位符，须由管理员预先创建相应受保护目录。
每位保管人在自己的 0700 目录内独立生成一把密钥，再只交换 `.pub.json`：

```sh
python -m tools.nodelite_release keygen --private /offline/custodian/key.pem --public /offline/custodian/key.pub.json
```

汇总公开 key 文件，建立 policy（路径相对于 policy 文件）：

```json
{
  "roles": {
    "root": {"threshold": 2, "keys": ["root-a.pub.json", "root-b.pub.json", "root-c.pub.json"]},
    "targets": {"threshold": 2, "keys": ["release-a.pub.json", "release-b.pub.json"]},
    "snapshot": {"threshold": 1, "keys": ["snapshot.pub.json"]},
    "timestamp": {"threshold": 1, "keys": ["timestamp.pub.json"]}
  }
}
```

```sh
python -m tools.nodelite_release root-draft --policy /ceremony/policy.json --out /ceremony/root.unsigned
```

各保管人检查 key ID、角色、阈值、版本、有效期及完整载荷后，分别生成 approval：

```sh
python -m tools.nodelite_release approve --metadata /ceremony/root.unsigned --private /offline/custodian/key.pem --out /ceremony/root-a.approval
python -m tools.nodelite_release assemble --unsigned /ceremony/root.unsigned --approval /ceremony/root-a.approval --approval /ceremony/root-b.approval --out /ceremony/roots/1.root.json
```

`roots/` 保留连续的 `1.root.json` 到当前版本，最多 128 份；不能删除历史并从 1 重建。
通过独立可信渠道记录初始 root 的 SHA256，不能从同一不可信下载源取得 root 和其“可信摘要”。

## 发布与续签

产物必须来自已审查、已测试的构建；两位发布审批人分别核对目标摘要、架构、渠道、版本及
单调序号。显示版本不用于防降级。一个仓库只服务一个 channel/architecture，stable/testing
使用不同仓库，生产建议分离密钥。目标不超过 32 MiB、平面名称固定。

```sh
python -m tools.nodelite_release targets-draft --artifact /build/nodelite-agent --channel stable --target x86_64-unknown-linux-musl --version 3.0.5 --sequence 2 --floor 1 --metadata-version 2 --out /ceremony/targets.unsigned
```

对 targets 收集两份 approval，用 `assemble --root /ceremony/roots/1.root.json` 合并。
然后分别生成、审批、合并 snapshot 与 timestamp：

```sh
python -m tools.nodelite_release role-draft --role snapshot --child /ceremony/targets.json --root /ceremony/roots/1.root.json --metadata-version 2 --out /ceremony/snapshot.unsigned
python -m tools.nodelite_release role-draft --role timestamp --child /ceremony/snapshot.json --root /ceremony/roots/1.root.json --metadata-version 2 --out /ceremony/timestamp.unsigned
python -m tools.nodelite_release publish --roots /ceremony/roots --targets /ceremony/targets.json --snapshot /ceremony/snapshot.json --timestamp /ceremony/timestamp.json --artifact /build/nodelite-agent --destination /srv/nodelite-repository
```

`role-draft` 的 child 必须是已签名文件，不能使用 unsigned 草稿。上述命令之间的审批/合并步骤
不能跳过。重新生成草稿意味着新的签名载荷，不能沿用旧 approval。

各角色版本独立递增：timestamp 可以引用未改变的 snapshot，仅用 timestamp 密钥续签；
snapshot 可引用未改变的 targets，随后生成新 timestamp。不需要为短期 timestamp 重签 targets。
建议 timestamp 不晚于每 6 小时刷新，snapshot 在 7 天到期前刷新，targets/root 在各自到期前
完成离线审批。此处只是运行规程，本轮未安装生产定时任务。过期、缺失或网络失败会拒绝更新，
不停止健康的旧 Agent。

发布器在进程锁内校验完整根链、签名阈值、有效期、子元数据长度/摘要、产物字节及防降级下限。
所有校验通过后生成不可变 `generations/<id>/`，fsync 后原子替换相对 `current` 链接。
它保留历史公开元数据和目标，以容忍刷新期间的混合 HTTP 请求；相同版本不同内容被拒绝。
私钥、approval 工作区和口令均不属于 web root。HTTP 服务用户只读，发布用户不能复用 Agent UID。

生产只暴露 `current/metadata/`、`current/targets/` 的只读 HTTPS 内容，不暴露发布工作区。
首次包和 root 摘要应另走可信渠道。远端明文 HTTP、重定向或带凭证/参数的 URL 不被节点接受。
默认保留最多 64 个发布代次，达到上限拒绝发布而不自动删数据。不能通过删除 current 或整个
仓库“修复”版本冲突。仓库、归档及所有祖先目录必须由 root 或发布者保护，其他 UID 不可
重命名或写入；仅把叶目录设为 0700 不足以保护整条路径。

## 到期告警与可恢复归档

定时调用检查命令，root 指纹来自此前独立确认的记录，而非从待检查镜像临时下载：

```sh
python -m tools.nodelite_release check --destination /srv/nodelite-repository --root-sha256 TRUSTED_ROOT_SHA256
```

退出码 0 表示检查通过，2 表示需要维护，1 表示输入或验证被拒绝。检查验证完整签名链、产物
摘要和公开文件范围；root/targets/snapshot/timestamp 分别在剩余 30 天/14 天/2 天/6 小时时
告警，过期不会误报 ready。达到 48 代次也告警。应将非零退出接入管理员告警；该命令不会
续签，仍需按上文安排在线角色刷新和离线审批，不能把检查任务当作刷新任务。

归档目录预先设为发布者所有的 0700，位于同一文件系统、仓库外、非 web root。先检查计划：

```sh
python -m tools.nodelite_release collect --destination /srv/nodelite-repository --archive /private/nodelite-archive --keep 3 --minimum-age 3600
```

确认后追加 `--apply`。默认不修改数据，至少保留两代（默认三代），至少给读者一小时宽限；
实际宽限应大于 HTTP 缓存/长连接存活时间。固定目录 FD 的原子移动只归档旧代次，不递归
删除；权限恢复使用已打开的 inode，不跟随可替换路径。当前代次保留所有历史公开根、版本
元数据和产物，不因归档旧代次丢失根链。发布/归档使用同一排他锁，检查/组包持共享锁。

归档内容仍占磁盘，需另设人工复核的备份和容量管理；本工具不清理当前代次内的历史产物。
进程在移动窗口被杀可能留下 0755 目录；它不暴露私钥（该目录仅含公开文件），后续检查会
拒绝非固定权限，需人工核对、恢复 0555，而不是删除状态绕过检查。

## 换钥、吊销与恢复

修改公开 policy 后，使用 `root-draft --previous N.root.json` 生成 N+1。审批必须同时满足旧 root
阈值和新 root 阈值，合并也必须传 `--previous`；旧角色密钥只保留在被替换的历史 root 中。
用新角色密钥重新签署 targets/snapshot/timestamp，再发布连续根链。节点持久化新 root 后，
镜像隐藏新文件也不能恢复被吊销的旧密钥。

丢失一把 2-of-3 root 密钥时，由剩余两位保管人完成换钥。targets 密钥丢失/泄露时，经 root
审批替换该角色，然后重新发布。若攻击者已掌握 root 阈值，或保管人无法再满足旧 root 阈值，
不能声称能在线自救：需要事件响应、独立分发新的信任锚及本地管理员重新确认。

正常的新节点 enrollment 包使用当前最新已验证 root，避免多年根链轮换超过单轮刷新上限。
失败迁移重试必须保留原 anchor，可显式 `pack-enrollment --bootstrap-version N`；这不是允许已
运行的 updater 降级信任根。首次迁移包同时含 root 管理的 updater 和固定单元，管理员必须审查
完整包 SHA256，不能仅凭签名 Agent 的摘要信任其中的 root 程序。

## 给一键安装器发布迁移包（仅维护者操作）

仍使用 `pack-enrollment`，加上固定仓库 URL 即输出自包含公开归档：

```sh
python -m tools.nodelite_release pack-enrollment \
  --repository /srv/nodelite-repository \
  --updater /build/nodelite-updater-x86_64-unknown-linux-musl \
  --units nodelite-updater/systemd \
  --metadata-url https://updates.example.invalid/stable/x86_64/metadata/ \
  --targets-url https://updates.example.invalid/stable/x86_64/targets/ \
  --out /review/nodelite-enrollment-x86_64-unknown-linux-musl.tar.gz
```

归档只含 17 个公开登记文件、manifest、固定登记描述和标准库迁移／恢复代码，不含签名
SDK、测试、私钥、口令或生产配置；旁边生成 `.sha256`。逐架构使用匹配的仓库和正常 updater
构建（禁止 `lab-faults`），独立核对 root、URL、平台及完整归档摘要后，上传到**同一具体
Release**，将 `.sha256` 条目并入该 Release 原有 `SHA256SUMS.txt`，保留全部既有条目。
每个资产文件名只保留一个摘要；重新组包时替换该项，不要重复追加。
新安装器和 `install-agent-signed.sh` 必须同版且都在清单中；发布工作流负责正常 updater、
helper 和原有资产，签名包由可信发布机审批后另行上传，不把签名私钥交给 GitHub CI。

先确认签名源、完整连续根链及当前授权 baseline 可访问，再开放迁移入口。没有迁移包的
Release 仍能正常手动安装／升级；请求开启签名模式会明确失败，不修改已有 Agent。
