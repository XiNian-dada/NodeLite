---
title: 开发
nav_order: 10
---

# 开发

贡献流程和提交规范见 [CONTRIBUTING.md](https://github.com/XiNian-dada/NodeLite/blob/main/CONTRIBUTING.md)，安全问题请按 [SECURITY.md](https://github.com/XiNian-dada/NodeLite/blob/main/SECURITY.md) 私下报告。

## 构建与测试

需要 Rust（2024 edition）以及 Node.js 22 + pnpm（构建 Web 面板）。

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo +stable fmt --all --check

pnpm --dir nodelite-server/web install
pnpm --dir nodelite-server/web test
pnpm --dir nodelite-server/web lint
pnpm --dir nodelite-server/web typecheck
```

Server 会把 `nodelite-server/web/dist` 嵌入二进制，构建 Server 前会自动构建前端。

构建 Linux 静态二进制：

```bash
cargo build --release --target x86_64-unknown-linux-musl -p nodelite-server -p nodelite-agent
cargo build --release --target aarch64-unknown-linux-musl -p nodelite-server -p nodelite-agent
```

协议模糊测试：

```bash
cargo test --manifest-path fuzz/Cargo.toml
cargo run --manifest-path fuzz/Cargo.toml --bin protocol_messages -- 10000
```

压测见[性能基准](benchmarks.md#自己运行)。

## 目录结构

| 目录 | 内容 |
| --- | --- |
| `nodelite-proto/` | 共享类型、WebSocket 消息、配置解析与校验 |
| `nodelite-agent/` | 指标采集、连接上报、凭据保存 |
| `nodelite-server/` | 后端：HTTP / WebSocket、注册表、历史库、认证、告警；`benches/` 为压测 |
| `nodelite-server/web/` | Vue 3 前端，见[前端说明](https://github.com/XiNian-dada/NodeLite/blob/main/nodelite-server/web/README.md) |
| `nodelite-updater/` | 可选的签名 Agent 更新器 |
| `tools/` | 发布与迁移用的 Python 工具（只在发布和首次启用签名更新时使用） |
| `scripts/` | 安装、升级与 CI 脚本及其测试 |
| `config/` | 示例配置 |
| `ops/` | Prometheus 与 Grafana 示例 |
| `docs/` | 本文档站 |
| `fuzz/` | 协议模糊测试 |

单元测试通常紧邻实现，跨模块集成测试放在各 crate 的 `tests/` 目录。

修改签名或迁移工具时：

```bash
python3 -m venv .venv
.venv/bin/python -m pip install --only-binary=:all: -r tools/nodelite_release/requirements.txt
.venv/bin/python -m unittest discover -s tools -t . -p 'test_*.py' -v
```

## 技术说明

实现细节记录在[技术说明](notes.md)中。
