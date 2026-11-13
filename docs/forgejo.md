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
| Coverage | 保留 75% 行覆盖率门槛，保存 XML/HTML 制品；不再上传 Codecov |
| Release | 版本 tag 或从对应 tag 手动触发后验证并构建 Linux x86_64/aarch64 musl 的 Server、Agent、Updater、SBOM 和校验清单；上传完整草稿后再发布 Forgejo Release |
| Documentation site artifact | 构建 Jekyll 文档，生成 `nodelite-docs-site` 制品中的 `nodelite-docs.tar.gz`；尚未部署公开站点 |

`mainland-compiler` 是 Linux amd64 Runner。Rust 作业使用编译机上的 `nodelite-ci:20261113-v1` 专用 Debian 13 镜像（构建源见 `scripts/ci/Dockerfile`），Rust 固定为 1.93.1，显式限制 Cargo 编译并发为 2。前端依赖仍由现有 pnpm lockfile 固定，Actions 引用固定为完整 commit SHA。制品使用 Forgejo 支持的 v3 协议；musl 交叉编译通过固定版本 cargo-zigbuild 和 Zig 完成，无需向作业开放 Docker socket。

`systemd-acceptance` 在普通 Runner 内启动一次性 QEMU Debian 虚拟机，验证真实 systemd 升级恢复和流控沙箱，无需 privileged 作业或宿主机 Docker socket。虚拟机固定 2 vCPU、1 GB 内存，使用独立 overlay，结束后清理；失败保留串口日志。macOS CI 已按要求关闭，原配置保存在 `.github/legacy-workflows/ci.yml`；当前 Forgejo Release 不产出 macOS 二进制。

Forgejo Release 的 `BUILD_INFO.json` 记录 source commit、tag 和仓库，属于构建记录，不是 GitHub OIDC 签名证明。原安装器的 GitHub attestation 校验保持原有拒绝策略；开启该校验的部署不能用 Forgejo 构建记录替代。安装器和运行中 Server 的默认 GitHub 更新 URL 尚未改变，使用 Forgejo Release 需要另外配置下载源与私有仓库认证。

## 签名 Agent 更新

迁移期间签名 Agent 更新停用，文档制品不包含更新元数据。`updates/github.json`、离线批准内容和原签名校验代码保留。恢复前必须找回原状态分支、角色私钥及已批准的元数据，并明确新的更新发布 URL；不能通过初始化新状态、关闭签名验证或替换信任根绕过已有部署的连续性校验。此迁移未触发 Release，也未恢复定时签名或发布站点。

源码按 `forgejo.sha` 从 Forgejo API 下载归档，使用同实例的作业 token 和现有 `SOURCE_ARCHIVE_PROXY` 下载路线。编译环境的工具安装不修改 Forgejo 主站容器。
