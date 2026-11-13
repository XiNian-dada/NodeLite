---
layout: default
title: Forgejo 协作与工作流
nav_order: 15
---

# Forgejo 协作与工作流

当前主仓库是 [XiNian-dada/NodeLite](https://akyuu.leeinx.com/XiNian-dada/NodeLite)，为私有仓库。本地 `forgejo` 远端使用现有 SSH 凭据；GitHub 远端保留作历史参考。迁移只上传了 `main` 的完整历史，其他本地工作分支、未跟踪文件、旧 Release 附件和 GitHub Issue/PR 未自动迁移。

工作流位于 `.forgejo/workflows/`。原工作流保存在 `.github/legacy-workflows/`，避免 Forgejo 加载旧配置或重复运行。

| 工作流 | 当前行为 |
| --- | --- |
| CI | Rust fmt、clippy、workspace/MSRV 测试、前端 lint/typecheck/单测、浏览器及真实 Server E2E、fuzz smoke、安装器、依赖审计与 musl 交叉编译 |
| Native macOS Agent CI | 在无头 macOS Intel 客体执行 proto/Agent 单测、Agent clippy、Intel/ARM64 构建与 Intel 实际采集；仅 main push 或手动触发 |
| Coverage | 保留 75% 行覆盖率门槛，保存 XML/HTML 制品；不再上传 Codecov |
| Release | 版本 tag 或从对应 tag 手动触发后验证并构建 Linux x86_64/aarch64 musl 的 Server、Agent、Updater、SBOM 和校验清单；上传完整草稿后再发布 Forgejo Release |
| Documentation site artifact | 构建 Jekyll 文档，生成 `nodelite-docs-site` 制品中的 `nodelite-docs.tar.gz`；尚未部署公开站点 |

`mainland-compiler` 是 Linux amd64 Runner。Rust 作业使用编译机上的 `nodelite-ci:20261113-v2` 专用 Debian 13 镜像（构建源见 `scripts/ci/Dockerfile`），Rust 固定为 1.93.1，最低支持版本为 1.89（修复 SQLite 捆绑库漏洞所需的 rusqlite 0.41 要求），显式限制 Cargo 编译并发为 2。前端依赖仍由现有 pnpm lockfile 固定，Actions 引用固定为完整 commit SHA。制品使用 Forgejo 支持的 v3 协议；musl 交叉编译通过固定版本 cargo-zigbuild 和 Zig 完成，无需向作业开放 Docker socket。

`systemd-acceptance` 使用仓库专属的 `nodelite-systemd` Runner，在现有 DinD 内启动专用 Docker 作业，通过独立 PID/mount/cgroup 命名空间启动真实 systemd。仅该作业增加 `SYS_ADMIN` 和 `NET_ADMIN` 能力，专属 Runner 并发为 1、每个作业 2 CPU/3 GB，普通编译作业不变；不使用 privileged 模式、宿主机挂载或 Docker socket。测试结束后容器由 Runner 回收，失败保留日志。macOS Agent 已恢复为 Linux 交叉编译：同一作业顺序构建 Intel x86_64 和 Apple Silicon ARM64，生成并校验 Mach-O 产物，最低系统目标为 macOS 11。CI 保存 `macos-agent` 制品，Release 包含两份 Agent 与各自 SHA-256。此作业不执行 macOS 单元测试或 launchd 测试。

Forgejo Release 的 `BUILD_INFO.json` 记录 source commit、tag 和仓库，属于构建记录，不是 GitHub OIDC 签名证明。原安装器的 GitHub attestation 校验保持原有拒绝策略；开启该校验的部署不能用 Forgejo 构建记录替代。安装器和运行中 Server 的默认 GitHub 更新 URL 尚未改变，使用 Forgejo Release 需要另外配置下载源与私有仓库认证。

## 签名 Agent 更新

迁移期间签名 Agent 更新停用，文档制品不包含更新元数据。`updates/github.json`、离线批准内容和原签名校验代码保留。恢复前必须找回原状态分支、角色私钥及已批准的元数据，并明确新的更新发布 URL；不能通过初始化新状态、关闭签名验证或替换信任根绕过已有部署的连续性校验。此迁移未触发 Release，也未恢复定时签名或发布站点。

源码按 `forgejo.sha` 从 Forgejo API 下载归档，使用同实例的作业 token 和现有 `SOURCE_ARCHIVE_PROXY` 下载路线。编译环境的工具安装不修改 Forgejo 主站容器。

macOS 交叉编译镜像为编译机本地的 `nodelite-macos-cross:20261113-v1`，复用 Rust 1.93.1，固定 cargo-zigbuild 0.24.1 / Zig 0.13.0，并使用本机 Xcode 的 macOS 26.5 SDK。SDK 不进入 Git 或公开镜像仓库。ARM64 产物在当前 Mac 上通过 `--help` 启动检查；这不等同于完整的 macOS 运行测试。

原生 macOS Runner `nodelite-macos` 是仓库专属的 `macos-x64:host` 环境，运行 macOS 14.8.9、SDK 15.2 / clang 16、Rust 1.93.1 与 Node 22.23.2。QEMU/KVM 容器不开放网页或 VNC，只经编译机 loopback SSH 管理；Runner 由 LaunchDaemon 自动启动，无需桌面登录。客体分配 2 vCPU/4 GiB、并发 1，保留系统 Frameworks 以运行真正的 macOS API。Intel 客体不能执行 ARM64 程序，因此 ARM64 仍只验证构建。

Host 作业可以访问专用 CI 账户的持久文件，文件权限不能隔离该账户运行的不可信代码。原生工作流不接收 PR，仅运行受信任的 main push 和手动任务。下载归档仍绑定 `forgejo.sha`，在客体直接访问同一 Forgejo；Cargo 关闭 HTTP 多路复用。每个作业开始前检查至少 6 GiB 空闲，将 target 放在 Runner 临时目录，成功或失败后均清理该次构建输出。SDK、客体磁盘和 Runner 凭据不进入仓库。
