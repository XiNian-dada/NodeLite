# NodeLite - AI 开发指南

本文档为 AI 辅助开发工具提供项目上下文和开发约定。仓库根目录的 `AGENTS.md`
是更高优先级的流程规范；开始任务时先读 `AGENTS.md`，再读本文。

## 项目概述

NodeLite 是一个轻量级 Rust 监控系统，采用 Server-Agent 架构。

核心特性：
- WebSocket 实时通信
- Token 认证，可选 TOTP 2FA
- SQLite 历史数据、审计日志与告警状态存储
- Vue 3 Web UI，由 server 嵌入静态构建产物
- 低资源占用：按场景跟踪 Server/Agent 的 RSS、PSS、匿名内存与 cgroup 总预算

性能目标：
- 200 节点并发：18,677 指标/秒
- p95 延迟：< 5ms
- 内存：同机、同构建、同负载做回归比较；采集规范见 `docs/memory-measurement.md`

## 架构

### 工作区结构

```text
NodeLite/
├── nodelite-proto/    # 共享协议、消息类型、配置解析与校验
├── nodelite-agent/    # Agent 端采集、会话、配置读写与上报
└── nodelite-server/   # Server 端接入、存储、告警、审计、API 与 Web UI
```

### 数据流

```text
Agent -> WebSocket -> Server -> SharedState / NodeRegistry
                            -> HistoryStore / AuditLog / AlertRuntime
                            -> HTTP API / Web UI / Prometheus metrics
```

### 关键模块

`nodelite-proto`：
- `message.rs`：Agent 与 Server 间的 wire message。
- `snapshot.rs`：系统快照与指标结构。
- `config/`：raw config、默认值、公开配置类型与校验。

`nodelite-agent`：
- `main.rs`：薄入口，实际逻辑在 library 模块。
- `collector*.rs`：平台指标采集。
- `session.rs`：WebSocket 会话、重连与上报循环。
- `config_io.rs`：Agent 配置加载、写回与权限处理。

`nodelite-server`：
- `main.rs`：薄入口，仅调用 server library。
- `lib.rs`：模块装配与 CLI/server 分发入口。
- `startup.rs`：启动编排、配置加载、路由/后台任务初始化。
- `cli.rs`：server 子命令与 agent install/upgrade 脚本生成。
- `app_state.rs`：HTTP 与 WebSocket handler 共享的可克隆状态。
- `state.rs` 与 `state/`：运行态节点视图、缓存、会话控制、SQLite WAL 设置。
- `registry.rs` 与 `registry/`：节点注册表、token、auth、持久化与渲染模型。
- `ws.rs` 与 `ws/`：Agent/browser WebSocket 握手、协议分发、session 与刷新逻辑。
- `handlers/`：HTTP 路由；`api_routes`、`auth_routes`、`install_routes`、`page_routes`、
  `metrics_routes`、`settings/` 按职责拆分。
- `history.rs` 与 `history/`：SQLite schema、查询和后台 writer。
- `audit/`：审计事件类型、存储、查询和异步 writer。
- `alerts/`：告警规则、评估器、tracker、runtime 与 delivery。
- `admission.rs`：WebSocket 准入、限流与封禁。
- `geoip.rs`：GeoIP 数据读取与节点位置推断。
- `web_assets.rs`：嵌入并服务 `nodelite-server/web/dist`。

`nodelite-server/web`：
- Vue 3 + Vite + Pinia + Vue Router + vue-i18n。
- `src/views/`：页面级视图。
- `src/components/`：可复用 UI 组件。
- `src/stores/`：Pinia store 和轮询/状态管理。
- `src/composables/`、`src/lib/`、`src/ws/`：图表、格式化、WebSocket client 等共享逻辑。

## 代码约定

### 错误处理

强制要求：
- 生产代码禁止使用 `.unwrap()`。
- 生产代码禁止使用 `.expect()`，除非有充分理由并用注释说明。
- 使用 `?` 传播错误。
- 使用 `anyhow::Context` 添加错误上下文。
- 模块间公共 API 优先定义自己的错误枚举，避免把 `anyhow` 扩散到边界外。
- 测试代码可以使用 `.expect("clear error message")`。

示例：

```rust
let config = load_config()
    .context("failed to load server config")?;
```

### 安全要求

强制要求：
- 所有外部输入必须通过 `sanitize.rs` 或对应模块校验。
- 密码/token 比较使用 `subtle::ConstantTimeEq`。
- 所有 SQL 查询使用参数化绑定。
- 敏感文件/目录强制 0600/0700。
- 使用 `getrandom` 生成 token。
- 禁止硬编码凭证。
- 禁止字符串拼接 SQL。
- 禁止在日志中输出敏感信息。
- 禁止直接序列化未脱敏的 `ServerConfig` 或 `ReadonlyAuthConfig`。

示例：

```rust
use subtle::ConstantTimeEq;

if token_bytes.ct_eq(expected_bytes).into() {
    // 验证通过
}

conn.execute(
    "INSERT INTO nodes (id, name) VALUES (?1, ?2)",
    params![node_id, node_name],
)?;
```

### 并发模式

推荐模式：
- 使用 `Arc<RwLock<T>>` 或专用 state wrapper 共享状态。
- 使用 `tokio::spawn` 处理独立后台任务。
- 使用 RAII 管理连接许可、临时凭证和清理逻辑。
- 热路径计数优先使用 `AtomicU64` 等无锁原语。

```rust
pub struct AppState {
    config: Arc<ServerConfig>,
    registry: NodeRegistry,
    next_session_id: Arc<AtomicU64>,
}
```

### 测试要求

强制要求：
- 新功能必须有单元测试。
- Bug 修复必须有回归测试。
- 配置、认证、WebSocket、存储和安全边界需要覆盖错误路径。
- 前端组件或 store 改动应补充对应 `*.spec.ts`。

常用命令：

```bash
cargo test --workspace
cargo +stable fmt --all --check
cargo clippy --all-targets -- -D warnings
pnpm --dir nodelite-server/web test
pnpm --dir nodelite-server/web lint
pnpm --dir nodelite-server/web typecheck
```

## 版本与发布构建

- Workspace `Cargo.toml` 中的 `0.1.0` 是没有版本注入时的 fallback，不随每个 release tag 修改。
- 官方 release workflow 将 tag 去掉前导 `v` 后，通过 `NODELITE_BUILD_VERSION` 注入 Linux Server / Agent 和 macOS Agent；`Cross.toml` 负责把该变量传入 cross 容器。
- 当前使用注入版本的运行时路径只有 Agent 身份中的 `agent_version`（上报后显示在面板）和 Server 设置 API 的 `server_version`。两处均在变量缺失时回退到 `CARGO_PKG_VERSION`。
- 直接使用 `env!("CARGO_PKG_VERSION")` 的字符串不会自动得到 release tag。现有 Webhook `User-Agent` 就属于这种情况，不能用它判断官方二进制版本。
- 新增面向用户或运维的版本展示时，应复用对应的 build-version helper，而不是直接读取 Cargo package version。

## 依赖管理

添加依赖原则：
- 优先使用标准库和现有 workspace 依赖。
- 选择维护活跃、安全记录良好的 crate 或 npm 包。
- Rust 依赖版本按仓库现有风格固定。
- 审查许可证，确保兼容 MIT/Apache-2.0。

核心 Rust 依赖：
- `tokio`：异步运行时。
- `axum`：Web 框架。
- `rusqlite`：SQLite 绑定。
- `serde` / `serde_json`：序列化。
- `tracing`：日志。
- `rustls`：TLS，不使用 OpenSSL。
- `subtle`：常量时间比较。
- `getrandom`：CSPRNG。

核心前端依赖：
- `vue`、`pinia`、`vue-router`、`vue-i18n`。
- `vite`、`vitest`、`vue-tsc`、`eslint`、`playwright`。

## 性能要求

优化原则：
- 避免不必要的克隆，优先引用、`Arc` 或已有缓存。
- SQLite 写入使用事务和后台 writer。
- WebSocket 接入使用 admission controller 防止资源耗尽。
- Prometheus 与 dashboard 聚合应控制分配和基数。

## 文件大小限制

目标：
- 单个文件 < 500 行为理想状态。
- 单个文件 < 800 行为硬上限。
- `main.rs` < 200 行。

当前需要关注的高水位文件：
- `nodelite-server/src/handlers/metrics_routes.rs` 接近 800 行，上方继续增长前优先拆分 metrics 类型、渲染和测试。
- `nodelite-server/src/state.rs` 接近 800 行，上方继续增长前优先拆分 overview、registry、session control、cache 相关逻辑。
- `nodelite-proto/src/config/tests.rs` 和 `nodelite-agent/src/collector_linux.rs` 也接近上限，新增覆盖或平台逻辑时优先拆小。

不要再把 `nodelite-server/src/main.rs` 或 `registry.rs` 当作超大文件处理；它们已经拆分过。

## 提交规范

Commit message 格式：

```text
<type>(<scope>): <subject>
```

Type：
- `feat`：新功能。
- `fix`：Bug 修复。
- `docs`：文档。
- `refactor`：重构。
- `test`：测试。
- `chore`：构建/工具。
- `perf`：性能优化。
- `security`：安全修复。

Scope：
- `server`：服务端。
- `agent`：Agent 端。
- `proto`：协议定义。
- `auth`：认证模块。
- `ws`：WebSocket。
- `ui`：Web UI。

示例：

```text
fix(ws): avoid reconnect storm after token expiry
docs(server): refresh architecture guide
```

## 常见任务

### 添加新的 HTTP 端点

1. 在 `nodelite-server/src/handlers/` 下选择对应 route 模块。
2. 只在需要时新增子模块；优先复用 `api_routes`、`auth_routes`、`install_routes`、`settings/`。
3. 在 router builder 或模块导出处注册路由。
4. 为认证、权限和错误响应添加测试。
5. 更新 API 或开发文档。

### 添加新的 WebSocket 消息类型

1. 在 `nodelite-proto/src/message.rs` 定义 wire 类型。
2. 更新 `nodelite-server/src/ws/protocol.rs`、`ws/session.rs` 或相关 handler。
3. 更新 `nodelite-agent/src/session.rs` 的发送或处理逻辑。
4. 添加协议、server 和 agent 侧测试。

### 添加新的系统指标

1. 在 `nodelite-agent/src/collector*.rs` 添加采集逻辑。
2. 在 `nodelite-proto/src/snapshot.rs` 添加字段。
3. 在 `nodelite-server/src/sanitize.rs` 或对应模型校验新字段。
4. 在 Vue Web UI 的 store/component 中展示。
5. 添加 Rust 单元测试和前端 `*.spec.ts`。

### 修改配置项

1. 更新 `nodelite-proto/src/config/raw.rs` 的 raw section，字段使用 `#[serde(default)]`。
2. 更新 `nodelite-proto/src/config/defaults.rs`。
3. 更新公开 `ServerConfig` / `AgentConfig` 类型和校验。
4. 更新示例 TOML 或默认模板。
5. 添加缺省、非法值和迁移兼容测试。

### 修改前端 UI

1. 在 `nodelite-server/web/src/views`、`components`、`stores` 或 `composables` 中定位责任边界。
2. 与 API 类型保持同步，优先更新 `src/api/types.ts`。
3. 添加或更新相邻 `*.spec.ts`。
4. 运行 `pnpm --dir nodelite-server/web test`，必要时运行 `lint` 和 `typecheck`。
5. 若 server 需要嵌入新 UI，运行 `pnpm --dir nodelite-server/web build` 生成 `web/dist`。

## 调试技巧

启用详细日志：

```bash
RUST_LOG=debug cargo run -p nodelite-server
RUST_LOG=nodelite_server=trace cargo run -p nodelite-server
```

运行单个 Rust 测试：

```bash
cargo test test_name -- --nocapture
```

运行单个前端测试：

```bash
pnpm --dir nodelite-server/web test src/components/NodeList.spec.ts
```

性能分析：

```bash
cargo build --release
perf record ./target/release/nodelite-server
perf report
```

## 参考资料

- [AGENTS.md](AGENTS.md)：AI 工作流程和代码审查规范。
- [README.md](README.md)：用户文档。
- [SECURITY.md](SECURITY.md)：安全模型。
- [CONTRIBUTING.md](CONTRIBUTING.md)：贡献指南。
- [nodelite-server/web/README.md](nodelite-server/web/README.md)：前端开发说明。
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)。
