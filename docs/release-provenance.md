---
title: 发布来源验证
parent: 技术说明
---

# 发布来源验证

新版 release workflow 在发布前为二进制、安装器、安装器模块、SBOM 和 SHA256SUMS.txt
生成 GitHub artifact attestation。失败会阻止发布。Release 同时包含离线证明包
`provenance.intoto.jsonl`；它是签名证明的传输副本，无需信任其下载位置。证明包在摘要清单之后生成，由其签名验真，避免清单与证明循环依赖。

手动发布也必须从目标 tag 运行：`gh workflow run release.yml --ref "$TAG" -f tag="$TAG"`。
workflow ref 与输入 tag 不一致会在构建前拒绝，保证证明中的 source ref 对应实际发布 tag。

SHA-256 只验证下载内容与清单一致。攻击者同时替换资产和同源清单时，仅检查 SHA-256
无法识别替换。Attestation 把摘要绑定到 GitHub Actions 的签发身份、仓库、workflow 和 tag。
它仍信任仓库源码、发布工作流及 GitHub OIDC/Sigstore；不能抵御获准工作流本身被恶意修改。

先从可信渠道安装支持 `gh attestation verify` 的 GitHub CLI，并独立审核目标 tag/commit。
在执行下载的安装器前验证安装器本身，例如：

```sh
TAG=v1.2.3 # 替换为审核过的实际发布 tag
curl -fLO "https://github.com/XiNian-dada/NodeLite/releases/download/$TAG/install-server.sh"
gh attestation verify install-server.sh \
  --repo XiNian-dada/NodeLite \
  --signer-workflow XiNian-dada/NodeLite/.github/workflows/release.yml \
  --source-ref "refs/tags/$TAG" \
  --cert-oidc-issuer https://token.actions.githubusercontent.com \
  --deny-self-hosted-runners && \
NODELITE_VERIFY_ATTESTATIONS=1 NODELITE_SERVER_VERSION="$TAG" sh install-server.sh
```

用同样的命令可验证 `install-agent.sh`、二进制或 SHA256SUMS.txt。
下载证明包后可以给 `gh attestation verify` 添加 `--bundle provenance.intoto.jsonl`，
仍须保留上述身份约束。不要仅检查“存在签名”。

`NODELITE_VERIFY_ATTESTATIONS=1` 在 Server/Agent 安装和升级中强制验证每个即将执行的下载文件。
证明缺失、摘要/身份/tag 不匹配、gh 不可用或验证失败都拒绝执行和激活，不能自动回退到 SHA-only。
Web 升级在 Server 的 systemd unit 中设置 `Environment=NODELITE_VERIFY_ATTESTATIONS=1`；
Server 会把该选择传给沙箱启动器，再传给下载的安装器。按 gh 的要求准备认证；Web 沙箱不能读取主目录的 gh 认证文件，缺少验证所需认证时会失败并保留升级日志。

旧版本没有证明，为兼容现有部署默认值仍为 `0`，保留 SHA-256 检查。
建议新的受管部署启用强制验证。直接 `curl | sh` 无法独立验证初始脚本，
即使脚本内有验证选项也无法证明初始脚本可信。
本次改动不会替历史 release 补发证明；首次新版 tag 发布后才能验证真实构建证明。
