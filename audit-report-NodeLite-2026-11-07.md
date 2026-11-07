# NodeLite 全量代码审计报告（Fuck My Shit Mountain Audit Report）

**Project:** NodeLite

**Audit mode:** full（全部 25 个维度）

**Date:** 2026-11-07

**Reviewer:** Codex / GPT 主 Agent

**Language / Output:** 中文 / Markdown

**Baseline:** `main`，`ece3885a761453bc2a029b4ca1c19d9346301b4b`

**Repository:** `/Users/bernard/Code/NodeLite`

---

## 1. 执行摘要（Executive Summary）

NodeLite 的核心工程基础较完整：协议、采集、服务端和特权更新器分属四个 Rust crate；服务端已有认证、会话容量限制、输入校验、SQLite 写入队列、查询并发限制、诊断指标与分层测试。Vue 前端有统一 API 入口、状态仓库及实时消息处理；发布链路已有校验和、来源证明、签名更新及回滚设计。本次证据支持“需要修补局部边界的可维护项目”，不支持全面重写，也不足以证明生产环境完全安全。

共记录 **7 项 Medium 风险，其中 6 项 Confirmed、1 项 Suspected**，未发现证据充分的 Critical 或 High 问题。优先处理升级清单的路径语义偏差：暂存配置会把暂存目录中的 `passkeys.json` 纳入备份和恢复范围。其次是磁盘探测线程缺少全局预算、历史查询等待无上限、前端请求缺少应用级截止时间。敏感管理操作审计覆盖不足属于文档已承认、但仍有实际影响的设计限制。Vite 开发依赖存在已知中危公告；无认证回环部署的审计接口暴露则需要先明确公开面板与安全诊断的边界。

验证包括云端 Rust 全工作区测试 **939 通过、2 忽略**、全目标全特性 Clippy 通过；前端 **95 个文件 / 646 项测试**、类型检查、ESLint、构建均通过；Chromium 桌面夹具界面测试 **27 通过**。另用实际 Rust 模块、实际服务二进制和实际前端 API 模块做了四类隔离探针。覆盖采用“全部第一方目录系统盘点 + 高风险链路深读 + 静态扫描和可行测试”，并非逐字节审查 12.6 万行文本。真实生产负载、实际升级回滚、完整签名仪式、多平台硬件行为与浏览器连接真实后端的整套测试未在本次全部运行。

### 评分看板（Score Dashboard）

```text
Security        ████████░░  8.0  A
Stability       ███████░░░  7.4  A
Performance     ███████░░░  7.4  A
Testing         ████████░░  8.3  A
Maintainability ████████░░  8.4  A
Design          ████████░░  7.7  A
Release         ████████░░  7.6  A
─────────────────────────────────
Overall         ████████░░  7.8  A
```

分数越高越好，10.0 为最佳；七项分数为基于证据的判断，总分为七项平均值四舍五入至一位小数。A 表示基础良好、存在需关注的局部风险，不代表已通过生产安全认证。

| 评分维度 | 一句话依据 |
|---|---|
| Security | 认证、二次确认、签名更新与输入边界较完整，但 F05–F07 影响追溯、开发环境安全及隐私；覆盖 Medium，未验证真实部署配置。 |
| Stability | 写入队列、取消释放及故障信号已有保护，F01–F04 仍存在文件范围和资源等待问题；覆盖 Medium。 |
| Performance | 有批处理、缓存和查询执行并发限制，但 F02、F03 缺少全局资源预算；未重跑生产规模压测，覆盖 Medium。 |
| Testing | Rust、Vue 与浏览器夹具测试提供了实际回归保护，但安装器夹具漏掉真实清单字段，系统及 Python 验证仍有缺口；覆盖 Medium。 |
| Maintainability | 模块拆分、共享协议和现有工具复用较好，问题大多可局部解决；跨平台分支与全部函数复杂度未逐一审查，覆盖 Medium。 |
| Design | 状态职责和显式依赖总体清晰，F01 的路径契约、F02–F04 的资源生命周期需要补齐；覆盖 Medium。 |
| Release | CI、锁文件、来源校验和恢复机制齐备，但 F01 和 F06 应在相关版本发布前处理；未实测生产签名与 systemd 升级，覆盖 Medium。 |

### 发现统计（Finding Statistics）

| Severity | Count | Confirmed | Suspected |
|---|---:|---:|---:|
| Critical | 0 | 0 | 0 |
| High | 0 | 0 | 0 |
| Medium | 7 | 6 | 1 |
| Low | 0 | 0 | 0 |
| Info | 0 | 0 | 0 |
| **Total** | **7** | **6** | **1** |

Confirmed 表示具体缺陷或缺失已由源码、依赖解析结果或探针确认，不表示已经发生生产事故。F07 的无认证返回行为已复现，但是否违反产品预期访问策略尚不确定，因此整项保留为 Suspected。跨维度引用同一编号不重复计数。

## 2. 项目地图（Project Map）

| 区域 | 入口与职责 | 状态与边界 |
|---|---|---|
| `nodelite-proto` | 配置、共享消息、限制及校验 | Server 与 Agent 的协议和配置契约；不拥有服务端持久状态。 |
| `nodelite-agent` | 平台采集、连接与重连、凭据持久化、流量控制 | 本机指标与短期采样状态；经 WebSocket 向 Server 上报，特权操作受运行环境约束。 |
| `nodelite-server` | `startup.rs` 拼装 HTTP / WebSocket；Registry、History、Audit、Alerts | 注册与令牌状态、内存实时视图、SQLite 历史与审计、快照、Passkey 文件、配置文件分别持有各自状态。 |
| `nodelite-server/web` | Vue / Pinia / Router，统一 API 与实时消息组合函数 | 界面状态和服务端状态副本；浏览器输入不作为授权依据。 |
| `nodelite-updater` | policy、trust、download、activate、state | 签名信任链、版本下限、特权文件及服务切换；与网络下载和远程触发之间有独立授权边界。 |
| `tools` | 发布签名、迁移与维护工具 | 运营者的离线密钥、公开元数据、迁移目录；本次仅审查程序与合成测试数据。 |
| `scripts` / `.github` | 安装、升级、Web 更新、CI 和发布 | 下载来源 → 校验 → 停服备份 → 替换 → 健康检查 → 回滚，是 F01 的关键跨语言边界。 |
| `docs` / 根文档 / 配置示例 | 安装、开发、安全、配置、告警、升级、性能口径 | 用于核对对外承诺与实现；历史性能数字不直接充当本次实测。 |

主要数据流：本机采集 → Agent WebSocket → Server 认证和清洗 → Registry 内存视图 → 有界 History 写队列 → SQLite；浏览器通过 REST 初始化和 WebSocket 增量获得状态。告警从状态与历史中计算，经受控外部通道发送。管理请求经过认证和敏感操作确认，修改配置、节点或发起更新；这些操作的持久审计覆盖见 F05。

安全边界包括浏览器与 Server、Agent 与 Server、反向代理与可信客户端 IP、本地文件与运行身份、非特权 Agent 与特权 updater、发布者与下载端。项目中的 Agent 是监控采集进程，未发现产品运行时 LLM、RAG 或模型工具调用面。

### 盘点与范围说明

按 `git ls-files` 盘点 **783 个跟踪文件**，其中 **670 个文本文件，共 125,755 行**。主要区域：proto 38 文件 / 7,059 行，agent 40 / 7,614，server/src 188 / 50,464，web/src 204 / 33,456，web/e2e 19 / 1,400，updater 23 / 4,009，scripts 24 / 4,894，tools 29 / 3,683，docs 22 / 1,700，`.github` 9 / 1,601。上述区域统计是子集，其余包括根文件、独立测试和基准等。

排除 `.git`、依赖目录、`target`、`dist`、生成资源与二进制、历史 `output`、`.claude/worktrees` 等其他工作树。锁文件仅在依赖和发布检查中纳入。用户原有未跟踪文件 `NodeLite-architecture-report.html` 未作为当前主树源码，也未修改或纳入提交。全文搜索结果中的测试 `unwrap`、固定 SQL 片段、故障注入和注释标记均经语境区分，不直接当作漏洞。

### 覆盖矩阵（Coverage Matrix）

| Dimension | Coverage | Evidence inspected | Exclusions / limits |
|---|---|---|---|
| 架构 Architecture | Medium | 四 crate、startup、state、registry、history、updater、前端 store | 未做全量调用图及所有变更的历史耦合分析。 |
| 安全 Security | Medium | auth、step-up、sanitize、WS、文件权限、更新信任链、依赖扫描 | 未测试生产反代、真实账户、实际密钥和对公网渗透。 |
| 稳定性 Stability | Medium | deadline、队列、关闭、重连、writer、探针 | 未做长期故障浸泡、真实磁盘挂起和断电实验。 |
| 性能 Performance | Medium | 查询缓存/并发、采集线程、批处理、bundle | 未复测大集群吞吐、p95、RSS。 |
| 测试 Testing | Medium | Rust 939、Vue 646、浏览器 27、CI、脚本夹具 | 两个系统测试忽略；Python 缺依赖；live E2E 未重跑。 |
| 可维护性 Maintainability | Medium | 文件盘点、模块边界、错误类型、重复实现检索 | 未对全部函数运行复杂度分析。 |
| 设计 Design | Medium | 路径契约、状态归属、并发预算、API 生命周期 | 未逐一形式化所有状态机。 |
| 发布 Release | Medium | CI、release、install、回滚、签名工具、真实 manifest | 未执行生产部署、多平台 release 构建或实际回滚。 |
| 文档 Documentation | Medium | README、SECURITY、开发、配置、升级、性能、签名说明 | 未逐个执行全部文档命令和外链验证。 |
| 可观测性 Observability | Medium | metrics、readyz、审计、写入诊断、告警指标 | 未核实生产告警规则和通知送达。 |
| 配置安全 Configuration | Medium | Raw 配置、默认值、示例、运行时修改、manifest | 未读取用户实际生产配置或密钥。 |
| 数据完整性 Data Integrity | Medium | SQLite、迁移、原子写、快照、回滚范围 | 未做真实断电、全量备份恢复及大库升级。 |
| 隐私 Privacy | Medium | 登录 IP/UA、审计字段与保留、配置脱敏、删除 | 未核实部署者的数据治理政策及外部服务保留期。 |
| 可访问性 Accessibility | Medium | 原生 dialog、焦点、键盘、加载/错误、相关 E2E | 未运行读屏器、全部断点/浏览器和完整 WCAG 检查。 |
| 供应链 Supply Chain | Medium | Cargo/pnpm 锁、公告、Action 固定、TUF/来源证明 | 未验证真实发布资产、私钥分离或完整 Python 签名仪式。 |
| 成本 Cost | Medium | 线程、队列、缓存、保留、构建体积、外部调用 | 无实际云账单；未实测高规模资源曲线。 |
| AI/LLM 安全 AI Safety | Not assessed | 四 crate 与前端 manifest、第一方运行入口 | 产品无此运行时表面；开发工具配置不等于产品模型链路。 |
| 降级 Fallback | Medium | catch/unwrap_or 扫描、采集缓存、WS 回退、writer 降级 | 未对每个降级分支进行运行时故障注入。 |
| 测试真实性 Testing Authenticity | Medium | 安装器模拟清单、真实 Rust/浏览器分层、定向探针 | 未做全量 mutation testing。 |
| 类型安全 Type Safety | Medium | Rust unsafe/错误边界、serde、TS API 泛型及类型检查 | 未做 Miri、模糊测试和跨平台 ABI 验证。 |
| 前端状态 Frontend State | Medium | Pinia、usePolling、realtime、请求取消、组件测试 | 未覆盖所有视图同时操作与真实弱网组合。 |
| 后端 API Backend API | Medium | 路由、认证、验证、查询、分页、错误返回 | 未对全部端点做黑盒契约测试。 |
| 依赖重量 Dependency Weight | Medium | workspace manifest、pnpm、363 个 Cargo 锁包、bundle | 未做 cargo-bloat、每库 tree-shaking 或 release 体积归因。 |
| 代码一致性 Code Consistency | Medium | rustfmt、Clippy、ESLint、命名与错误处理抽查 | 未逐行统一语言/注释；不以审美差异判缺陷。 |
| 注释覆盖 Comment Coverage | Medium | 模块说明、取消/持久化/签名注释及公共 API 抽查 | 未计算机械注释率；未逐条核对全部文档注释。 |

### 实际验证与限制

所有退出码均来自实际命令结果，以下“通过”不代表超出该命令范围的环境也通过。云端工作目录为 `/home/compiler/NodeLite/audit-20261107-ece3885`，使用 Rust/Cargo 1.99.1、x86_64 Linux、`CARGO_BUILD_JOBS=2`。源码来自本次基线，前端 dist 为本次构建；`NODELITE_SKIP_WEB_BUILD=1` 仅跳过重复前端构建。

| 验证 | 命令 / 方法 | 结果与边界 |
|---|---|---|
| Rust 全量测试 | `NODELITE_SKIP_WEB_BUILD=1 CARGO_BUILD_JOBS=2 cargo test --workspace --locked --offline` | exit 0；939 passed、0 failed、2 ignored；含 1 项文档测试。忽略项要求隔离 systemd/network namespace。 |
| Rust lint | `NODELITE_SKIP_WEB_BUILD=1 CARGO_BUILD_JOBS=2 cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings` | exit 0，无项目 Clippy 警告。 |
| 服务二进制 | `NODELITE_SKIP_WEB_BUILD=1 CARGO_BUILD_JOBS=2 cargo build --locked --offline -p nodelite-server` | exit 0，debug 二进制用于隔离 HTTP 与 manifest 探针。 |
| Rust 格式 | `cargo +stable fmt --all --check` | exit 0。 |
| 前端单元测试 | `pnpm --dir nodelite-server/web test` | exit 0；95 文件、646 项通过。 |
| 前端静态检查/构建 | 分别执行 `pnpm --dir nodelite-server/web typecheck`、`lint`、`build` | 均 exit 0；本机 Node 23.11.0 不满足声明的 `^22.13.0 || >=24`，存在 engine 警告，未更换用户工具链。 |
| Chromium 界面 | `pnpm --dir nodelite-server/web exec playwright test --project chromium-desktop --reporter=line` | exit 0；27 通过。主要为 route-mocked 夹具；日志有未模拟请求的代理连接拒绝，不可据此宣称真实后端端到端通过。 |
| Cargo 公告 | `cargo audit --json` | exit 0；363 个锁定依赖、0 漏洞、无警告；公告库更新至 2026-11-06。 |
| pnpm 公告 | `pnpm --dir nodelite-server/web audit --json` | exit 1；6 个 moderate 包结果，5 条独立公告，0 high/critical；见 F06。 |
| Python 工具 | `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools -t . -v` | exit 1；20 项执行记录中含 4 个导入错误，本机缺 `cryptography`；不是 20 项业务测试全部失败。未联网安装依赖。 |
| 脚本静态检查 | `sh scripts/check-actions-pinned.sh`、`sh scripts/check-demo-credentials.sh`；对 CI 所列安装/更新/校验脚本执行 `shellcheck` | exit 0；不等于安装时的真实 systemd 行为已验证。 |
| 部分安装器测试 | 云端依次执行 `test-install-server-summary.sh`、`test-install-server-modules.sh`、`test-install-server-config.sh` | 前两项 exit 0，modules 报告 6 个 helper 用例通过；第三项因 Python 缺 `tomllib`/`tomli` exit 1；后续串行脚本未执行。 |
| 定向探针 | 实际 query limiter、mount probe；Vite SSR 实际 API；真实 Server 的 manifest/HTTP | 四类行为均获得可重复观察，详见 F01–F04、F07；使用合成数据，无生产请求。 |

Rust 测试分布：Agent 单元 70 + 集成 14；Proto 单元 129 + 契约 1 + 文档 1；Server 681；Updater 43。未把 ignored、构建成功、夹具 UI 或源码检视折算为额外“通过测试”。

## 3. 优先风险（Top Risks）

| 顺序 | 编号 / 标题 | Severity / Status | 影响与适用条件 |
|---:|---|---|---|
| 1 | F01 暂存配置污染升级备份路径 | Medium / Confirmed | 升级或初装失败恢复可能触碰临时目录下无关的同名 Passkey 文件。 |
| 2 | F02 磁盘探测缺全局线程预算 | Medium / Confirmed | 多挂载点同时阻塞时，单轮截止时间无法限制仍存活的线程总量。 |
| 3 | F03 历史查询等待无上限 | Medium / Confirmed | 慢查询与大量不同缓存键请求叠加时，执行限流后仍可积压大量等待任务。 |
| 4 | F04 前端 API 没有总截止时间 | Medium / Confirmed | 持续悬置的网络响应使共享设置 store 一直 busy，后续刷新被跳过。 |
| 5 | F05 敏感管理操作缺持久审计 | Medium / Confirmed | 改密码、关闭 2FA、删节点及 Server 更新无法依靠审计库完整追溯。 |
| 6 | F06 Vite 开发依赖存在中危公告 | Medium / Confirmed | 可到达开发服务器时，已知文件访问拒绝规则绕过尚未修补；不涉及生产嵌入式 dist 服务。 |
| 7 | F07 无认证模式返回审计详情 | Medium / Suspected | 回环监听通过无额外认证的反代公开时，可能同时公开登录 IP、UA 等审计数据。 |

## 4. 详细发现（Detailed Findings）

### Finding: F01 暂存配置的物理位置被误用为升级后的数据位置

- Severity: Medium
- Confidence: High
- Category: Release / Data Integrity
- Status: Confirmed
- Affected area: Server 安装、升级文件清单与回滚。
- Evidence:
  - File: [install-server-upgrade.sh:73](/Users/bernard/Code/NodeLite/scripts/install-server-upgrade.sh:73)，`prepare_upgrade_config` 在 `${TMPDIR:-/tmp}` 生成配置，再用这个路径调用真实 `upgrade-manifest`；[同文件:105](/Users/bernard/Code/NodeLite/scripts/install-server-upgrade.sh:105) 合并新旧路径，只排除暂存配置本身。
  - File: [upgrade.rs:33](/Users/bernard/Code/NodeLite/nodelite-server/src/upgrade.rs:33) 的 `render_manifest` 纳入派生的 Passkey 路径；[passkeys.rs:444](/Users/bernard/Code/NodeLite/nodelite-server/src/passkeys.rs:444) 使用 `config_path.with_file_name("passkeys.json")`。
  - File: [install-server-upgrade.sh:229](/Users/bernard/Code/NodeLite/scripts/install-server-upgrade.sh:229)，`restore_upgrade_entry` 会先删除清单中的路径，再恢复已有归档或确认 absent 标记。
  - Function / Module: `prepare_upgrade_config` → `render_manifest` → `backup_upgrade_files` → `restore_upgrade_entry`。
  - Relevant behavior: 同一份配置内容，真实二进制在 `probe-fixture/config/server.toml` 下输出 `config/passkeys.json`，在 `probe-fixture/staging/nodelite-next-config.fixture` 下输出 `staging/passkeys.json`；默认安装器因此会额外计算 `/tmp/passkeys.json`。
- Problem: 配置内容的暂存位置与最终运行位置被混为一谈，导致文件清单超出该次安装应当管理的范围。普通升级的旧清单仍包含真实 Passkey 文件，所以不能把此问题表述为“所有升级都漏备份真实 Passkey”。
- Why it matters: 安装器以高权限执行；错误纳入的文件可能被备份、覆盖或删除。已经存在的无关文件会被纳入归档；若备份时不存在、恢复前另一个进程创建同名文件，absent 恢复会删除它。损坏后果依赖文件存在及失败回滚时机，尚未观察到实际用户数据丢失。
- Realistic failure scenario: 使用默认 TMPDIR 升级，备份清单记录 `/tmp/passkeys.json` 不存在；升级健康检查失败前，其他工具创建该文件；回滚按清单执行删除。另一个场景是已有同名文件在升级期间被其他工具更新，回滚将它恢复成较旧版本。
- Minimal fix: 让 manifest 接口区分“从哪里读取暂存配置”与“最终配置路径”，并始终基于最终路径推导相邻数据文件；不要仅过滤一个固定的 `/tmp/passkeys.json` 字符串。
- Better long-term fix: 保留现有事务式升级架构，为清单建立显式路径语义契约；本问题不要求重写安装器。
- Regression test suggestion: 用真实 Rust manifest 配合隔离配置目录生成新旧清单；断言暂存目录哨兵文件不在备份/恢复集合，并模拟成功与失败恢复。现有 [upgrade_server_fixture.py:24](/Users/bernard/Code/NodeLite/scripts/tests/upgrade_server_fixture.py:24) 的模拟 manifest 未包含 Passkey 项，应补上契约交叉检查。
- Estimated effort: 2–4 小时，含清单接口、调用点和回归夹具。

### Finding: F02 磁盘探测仅限制每挂载点，未限制全局并发线程

- Severity: Medium
- Confidence: High
- Category: Stability / Performance / Cost
- Status: Confirmed
- Affected area: Agent 文件系统容量探测。
- Evidence:
  - File: [mount_probe.rs:71](/Users/bernard/Code/NodeLite/nodelite-agent/src/collector/mount_probe.rs:71)，每个可探测路径调用 `std::thread::Builder::spawn`；[同文件:111](/Users/bernard/Code/NodeLite/nodelite-agent/src/collector/mount_probe.rs:111) 的共享 deadline 仅限制收集结果的等待；[同文件:149](/Users/bernard/Code/NodeLite/nodelite-agent/src/collector/mount_probe.rs:149) 保留仍 pending 的消失挂载点。
  - File: [collector_linux/disks.rs:59](/Users/bernard/Code/NodeLite/nodelite-agent/src/collector_linux/disks.rs:59)，全部候选先探测，设备去重在结果返回后进行。
  - Function / Module: `MountProbes::probe_all`、`retain_mounts`、`collect_disks`。
  - Relevant behavior: 引入实际源模块，给 32 个不同路径提供可释放的阻塞探针；一轮截止后观测 `simultaneously_blocked_probe_threads=32`、`unavailable_results=32`，随后全部释放收尾。
- Problem: 单路径 pending 防重入和 2 秒等待上限可以避免同一路径不断产生新线程，但不同挂载点没有共享资源上限。协议输出条目截断发生在采集之后，也不能限制已经创建的线程。
- Why it matters: 每线程配置 64 KiB 栈仍需线程、地址空间和 PID 等资源；多个文件系统同时挂起或挂载路径持续变化时，资源总量可以继续增加。探针证明并发量没有全局上限，未证明生产已发生 OOM。
- Realistic failure scenario: 容器主机存在大量 bind mounts，或用户显式允许采集网络文件系统；多个 `statvfs` 调用挂起，下一轮又出现新的挂载路径，Agent 继续创建探测线程，最终接近进程/容器资源限额。
- Minimal fix: 在派生线程前取得固定数量的全局探测槽位；阻塞线程实际返回前不释放槽位。满额时复用旧值或标记不可用，并提供饱和/跳过指标；不能用无界 `spawn_blocking` 简单替换。
- Better long-term fix: 只有实际环境需要杀死不可中断探测时，再评估独立进程隔离；当前先补充固定预算即可。
- Regression test suggestion: 注入超过预算的不同挂载点和持续变更的路径；断言存活探测数不超过上限、单轮返回时间有界、已有成功值仍能使用，释放探针后槽位可恢复。
- Estimated effort: 0.5–1 天，含并发和恢复测试。

### Finding: F03 历史查询执行有界，但等待数量与等待时间无界

- Severity: Medium
- Confidence: High
- Category: Stability / Performance / Backend API
- Status: Confirmed
- Affected area: 历史查询准入与 HTTP 请求生命周期。
- Evidence:
  - File: [history/query_limit.rs:44](/Users/bernard/Code/NodeLite/nodelite-server/src/history/query_limit.rs:44)，无空闲 permit 时直接 `acquire_owned().await`，未限制 waiter 数量或等待时间。
  - File: [history.rs:559](/Users/bernard/Code/NodeLite/nodelite-server/src/history.rs:559)，缓存未命中后先等待 permit，再进入 `spawn_blocking`；[startup.rs:373](/Users/bernard/Code/NodeLite/nodelite-server/src/startup.rs:373) 的 HTTP 路由装配未提供统一请求队列上限/截止层。
  - Function / Module: `HistoryQueryLimiter::acquire`、`HistoryStore::acquire_query_permit`。
  - Relevant behavior: 实际源模块配置 limit=1 并持有 permit，再提交 32 个查询；200 ms 后 `permits_in_use=1, waiting=32, limit=1`。取消和释放后正常退出。
- Problem: `history_query_concurrency` 默认 4 限制的是执行查询数量；当数据库变慢时，新请求仍能无限加入 semaphore 等待队列。现有 waiting 指标和 RAII 释放机制有价值，但不构成准入限制。
- Why it matters: 请求、连接及捕获参数在队列中驻留，慢 SQLite 可转化为不断增加的内存与长尾延迟。不是 SQL 并发无限，也不能把 `busy_timeout` 等同于完整请求 deadline。
- Realistic failure scenario: 多个有权限的面板或脚本不断查询不同时间范围，缓存难以命中；少量磁盘慢查询占满执行槽位，后续 HTTP 请求在应用层积压。若反代有额外并发和超时保护，影响会减轻，但应用本身未规定上限。
- Minimal fix: 在执行 permit 之外增加有限等待准入和有界等待时间，饱和返回 429 或 503，并保证取消后计数恢复；状态码及重试行为需与前端约定。
- Better long-term fix: 根据慢查询实测再增加 SQLite 执行预算/中断机制；仅给 HTTP 外层套超时不能强制停止已进入阻塞线程的查询。
- Regression test suggestion: 占满执行槽位并提交超过队列容量的请求，断言新增请求快速拒绝、超时者退出、等待指标归零、查询完成后恢复；保留已有取消安全测试。
- Estimated effort: 约 0.5 天；若同时处理数据库执行中断则需额外验证。

### Finding: F04 统一前端 API 未设总截止时间，去重状态可持续卡住

- Severity: Medium
- Confidence: High
- Category: Stability / Frontend State
- Status: Confirmed
- Affected area: 通用 API、设置状态仓库和轮询恢复。
- Evidence:
  - File: [api/client.ts:38](/Users/bernard/Code/NodeLite/nodelite-server/web/src/api/client.ts:38)，`fetch` 只继承调用方参数，没有默认 deadline；响应体的 `text()` / `json()` 也无应用级总截止。
  - File: [stores/settings.ts:16](/Users/bernard/Code/NodeLite/nodelite-server/web/src/stores/settings.ts:16)，`loading` 为真时跳过后续刷新，仅在原请求结束的 `finally` 清除；[usePolling.ts:19](/Users/bernard/Code/NodeLite/nodelite-server/web/src/composables/usePolling.ts:19) 清理的是计时器，不会结束悬置请求。
  - Function / Module: `api<T>`、`useSettingsStore.refresh`、`usePolling`。
  - Relevant behavior: 用 Vite SSR 加载实际 API 模块，给 fetch 注入可释放 pending Promise；观测 `suppliedAbortSignal=false, pendingAfter100ms=true`。100 ms 仅用于证明调用没有传入默认中止信号，超时缺失结论同时依赖完整函数源码，并非声称 100 ms 就应失败。
- Problem: 默认调用没有可控完成时间；一直不结束的 fetch 或响应体读取使全局 loading 无法回收。组件生命周期中已有个别 AbortController，不能覆盖所有调用或替代时间截止。
- Why it matters: 用户无法通过普通刷新动作从悬置状态恢复，轮询也无法保证产生新结果；错误提示和可重试状态迟迟不出现。
- Realistic failure scenario: 代理接受请求后持续不返回完整响应，或返回 JSON 响应头后持续不结束 body；设置页一直加载，再次进入页面时共享 store 仍跳过刷新。
- Minimal fix: 在统一 API 层合并调用方取消信号与有限的请求 deadline，覆盖响应体读取，并在所有退出路径释放 timer/listener；区分导航中止与网络超时，确保 store 恢复可重试状态。
- Better long-term fix: 对特意长时间运行的操作采用任务状态查询或显式超时策略；写请求超时后结果可能未知，不能自动重放变更。
- Regression test suggestion: 分别模拟不返回响应头、不结束响应体、调用方取消和正常完成；断言 deadline 生效、loading 复位、第二次刷新可执行、写请求不会被自动重复提交。
- Estimated effort: 2–4 小时，含 API 和 store 回归测试。

### Finding: F05 高影响管理动作缺少对应的持久审计事件

- Severity: Medium
- Confidence: High
- Category: Security / Observability
- Status: Confirmed
- Affected area: 密码与 2FA 管理、删节点、Server 更新等操作。
- Evidence:
  - File: [audit/types.rs:15](/Users/bernard/Code/NodeLite/nodelite-server/src/audit/types.rs:15)，事件枚举覆盖登录、TOTP/Passkey 验证、连接、Token 误用、限流和 Agent 更新请求，未覆盖上述管理变更。
  - File: [settings/security.rs:74](/Users/bernard/Code/NodeLite/nodelite-server/src/handlers/settings/security.rs:74)、[同文件:454](/Users/bernard/Code/NodeLite/nodelite-server/src/handlers/settings/security.rs:454)，密码或禁用 2FA 持久化后无对应审计事件；[agent_delete.rs:85](/Users/bernard/Code/NodeLite/nodelite-server/src/handlers/settings/agent_delete.rs:85)、[updates.rs:115](/Users/bernard/Code/NodeLite/nodelite-server/src/handlers/settings/updates.rs:115) 仅有普通日志。
  - File: [agent_update.rs:124](/Users/bernard/Code/NodeLite/nodelite-server/src/handlers/settings/agent_update.rs:124) 已有可复用的 `NewAuditEvent` 写法；[SECURITY.md:125](/Users/bernard/Code/NodeLite/SECURITY.md:125) 明确承认操作审计尚不完整。
  - Function / Module: `change_readonly_password`、`disable_two_factor`、`delete_agent`、`start_server_update`。
  - Relevant behavior: 事件类型和调用点检索互相印证；不是“没有审计系统”，也不是新发现的鉴权绕过。
- Problem: 当前持久审计偏重登录与连接生命周期，高影响配置和节点管理动作不能形成与它们同等保留、可查询的审计记录。这是有实际代价的已知设计限制。
- Why it matters: 普通 journal/info 日志的保留、字段与查询机制不等同于审计 SQLite；事故后难以关联某次登录与关闭安全措施、删除节点、发起更新等动作。
- Realistic failure scenario: 共享管理员凭据被误用或账户失陷后，操作者关闭 2FA、删除节点；管理者查询默认保留 90 天的审计库，只能看到登录，缺少随后关键动作。共享身份本身仍限制个人归因，补事件不会把系统变成多用户平台。
- Minimal fix: 复用现有审计链路，优先记录密码/2FA/Passkey 管理、节点删除和 Server 更新，包含操作者标识、可信 IP、目标、结果及关联信息；只记录变更字段名，不写密码、token、TOTP seed 或告警秘密值。
- Better long-term fix: 在各写操作的明确提交点统一附加审计；是否要求失败时阻断管理动作需单独制定策略，不能悄然改变现有 best-effort 业务语义。
- Regression test suggestion: 对成功、拒绝、持久化失败分别断言事件类型和结果；等待审计 writer 完成后从数据库读取，并验证秘密字段未入库。
- Estimated effort: 0.5–1 天，先覆盖高影响操作，无需先引入完整 RBAC。

### Finding: F06 Vite 6.4.4 命中开发服务器文件访问绕过公告

- Severity: Medium
- Confidence: High
- Category: Security / Supply Chain / Release
- Status: Confirmed
- Affected area: 前端开发与测试依赖，不是生产 Rust 静态资源服务。
- Evidence:
  - File: [package.json:38](/Users/bernard/Code/NodeLite/nodelite-server/web/package.json:38)、[pnpm-lock.yaml:7](/Users/bernard/Code/NodeLite/nodelite-server/web/pnpm-lock.yaml:7)，声明/override 为 `^6.4.4`，锁定解析到 Vite 6.4.4；[vite.config.ts:16](/Users/bernard/Code/NodeLite/nodelite-server/web/vite.config.ts:16) 存在实际开发服务器入口。
  - Function / Module: `pnpm dev`、Vite 开发服务器；`pnpm audit --json`。
  - Relevant behavior: 公告 [GHSA-8g5g-mxqh-grgj](https://github.com/advisories/GHSA-8g5g-mxqh-grgj) 指出 query 可绕过 `server.fs.deny`，受影响版本为 6.4.4，修复版本为 ≥6.4.5；当前解析版本命中。
  - File: [ci.yml:281](/Users/bernard/Code/NodeLite/.github/workflows/ci.yml:281)、[release.yml:68](/Users/bernard/Code/NodeLite/.github/workflows/release.yml:68) 使用 `pnpm audit --audit-level high`，因此 moderate 公告不阻断绿灯。
- Problem: 开发服务器依赖已存在有修复版本的已知漏洞。构建和单元测试通过不能证明这一文件访问边界安全。
- Why it matters: 若攻击者能够到达开发服务器并构造相应请求，可能绕过开发者预期的文件拒绝规则。本次确认的是版本匹配与实际 dev surface，未对开发者敏感文件执行利用，也未证明某台生产主机暴露该端口。
- Realistic failure scenario: 开发者通过共享主机、容器端口映射或远程开发转发开放 Vite；仍依赖 `server.fs.deny` 保护工作文件，却运行受影响版本。
- Minimal fix: 升级到兼容的 Vite 修复版本并更新锁文件/override，运行前端测试、typecheck 和构建；同步处理可兼容升级的 Vitest 公告。CI 至少应把中危公告纳入有负责人和期限的处理流程。
- Better long-term fix: 按运行期与开发期攻击面分类追踪公告，避免只看 high 阈值，也避免对每个传递依赖强制跨主版本 override。
- Regression test suggestion: 在隔离 fixture 中验证开发服务器对受限文件的公告请求模式拒绝访问；重新运行 audit 确认目标公告消失，并保留生产构建和浏览器夹具测试。
- Estimated effort: 1–2 小时处理 Vite/Vitest 的兼容更新；传递依赖若需要上游升级另评估。

### Finding: F07 无认证回环模式下，审计详情的公开边界需要确认

- Severity: Medium
- Confidence: Medium
- Category: Privacy / Security / Backend API
- Status: Suspected
- Affected area: 无认证面板、反向代理公开部署与审计查询。
- Evidence:
  - File: [config/raw/server.rs:449](/Users/bernard/Code/NodeLite/nodelite-proto/src/config/raw/server.rs:449)，未启用认证仅允许回环监听，非回环监听会拒绝启动；这是已有的重要保护。
  - File: [middleware.rs:162](/Users/bernard/Code/NodeLite/nodelite-server/src/handlers/auth_routes/middleware.rs:162)，没有 `expected_authorization` 时中间件放行；[startup.rs:468](/Users/bernard/Code/NodeLite/nodelite-server/src/startup.rs:468)、[api_routes.rs:277](/Users/bernard/Code/NodeLite/nodelite-server/src/handlers/api_routes.rs:277) 的审计查询未另外要求已配置认证。
  - File: [audit/types.rs:61](/Users/bernard/Code/NodeLite/nodelite-server/src/audit/types.rs:61) 返回用户、IP、UA 和 details；相较之下 [healthz.rs:69](/Users/bernard/Code/NodeLite/nodelite-server/src/handlers/auth_routes/healthz.rs:69) 在无认证时明确拒绝内部诊断接口。
  - Function / Module: `evaluate_readonly_auth`、`audit_log`、`authenticated_readyz_diagnostics`。
  - Relevant behavior: 用真实二进制监听随机 loopback 端口，隔离 SQLite 插入合成记录；未带认证 GET `/api/audit-log` 返回 200 且包含合成用户/IP；同条件 `/api/readyz` 返回 401。
- Problem: 应用把“允许公开面板读取”扩展到了审计详情。行为已经确认；但项目没有明确要求无认证模式只公开非敏感指标，真实部署的外层认证也未知，因此风险认定保留为 Suspected，不能称为默认安装的公网鉴权绕过。
- Why it matters: 管理员登录来源、浏览器和事件时间可能与对外展示的节点状态有不同的隐私级别。开启外层认证或只在受信本机访问时，实际风险明显降低。
- Realistic failure scenario: 维护者曾启用认证积累登录记录，后来去掉应用认证来公开面板，继续用反代暴露回环端口；旧审计库尚在保留期，外部访客能够查询其中记录。
- Minimal fix: 明确产品策略；若无认证模式只用于公开仪表盘，应使 `/api/audit-log` 与内部诊断一致，要求配置并通过认证。若明确允许公开审计，应在模式说明中清楚列出公开字段及历史数据影响。
- Better long-term fix: 将公开指标与管理/安全数据放入明确的路由策略组，避免单个可选认证开关同时控制不同敏感度的数据。
- Regression test suggestion: 加入“无认证回环、有效认证、无效认证、移除认证但保留历史审计”的访问矩阵；按确定后的政策断言字段或状态码，确保不只是清空新测试库后误判无泄露。
- Estimated effort: 策略确认后 2–4 小时；策略确认本身不在代码工时内。

## 5. 架构分析（Architecture Concerns / Architecture Analysis）

- Coverage: Medium
- Inspected evidence: workspace manifest、`startup.rs`、`state/`、`registry/`、`history/`、`nodelite-updater/src/`、前端 stores 与 composables。
- Exclusions / limits: 未生成全量调用图，未依据历史提交量量化演进耦合。

四 crate 依赖方向和主要状态归属明确。Server 聚合 AppState 并将认证、存储、告警和 HTTP 处理拆成模块；虽然 `startup.rs` 集中装配路由，不足以据此认定为需要重写的“巨石”。实际架构问题是跨 Shell/Rust 的文件位置契约没有区分暂存与最终状态（F01），以及少数边界只限制局部资源，未规定总体预算。

### 架构摘要（Architecture Summary）

| Subtype | Count | Affected Areas | Recommended Action |
|---|---:|---|---|
| ModuleBoundary | 0 | 四 crate / Server 子模块 | 保持现有职责划分。 |
| DependencyDirection | 0 | 协议、采集、服务、更新 | 已检范围无须增加通用抽象层。 |
| StateOwnership | 0 | Registry、History、Audit、Pinia | 保持服务端为业务状态来源。 |
| BoundaryContract | 1 | F01：升级清单 | 显式传递最终配置路径。 |
| EvolutionRisk | 0 | 安装器与 manifest 的版本绑定 | 用契约测试约束跨语言变更。 |

- [x] 识别入口、持久化状态和主要信任边界。
- [x] 将真实跨边界失配与一般文件大小区分。
- [ ] 未验证所有未来功能扩展的变更影响范围。

## 6. 安全分析（Security Concerns）

- Coverage: Medium
- Inspected evidence: `auth/`、`handlers/auth_routes/`、`settings/security.rs`、`sanitize.rs`、WS admission、Passkey、更新信任链、依赖公告。
- Exclusions / limits: 无生产凭据、部署反代检查、完整渗透测试和私钥操作。

已确认的保护包括常量时间比较、CSPRNG 会话材料、认证会话容量、TOTP 重放/失败限制、浏览器 WebSocket Origin 校验、敏感写入确认及配置脱敏。查询的动态 SQL 片段主要用于固定谓词组合，值通过参数传入；没有把“存在字符串拼接”直接判为 SQL 注入。有限模式扫描未发现私钥块或典型 provider token，不能据此保证历史或所有文件没有秘密。

| 关联发现 | 安全结论 |
|---|---|
| F05 | 高影响操作的事后追溯能力不足，属于已知边界。 |
| F06 | 开发工具漏洞版本匹配，实际攻击面依赖开发端口可达性。 |
| F07 | 回环无认证 + 对外代理组合需明确数据访问策略；默认要求认证的公网监听不受该结论覆盖。 |

- [x] 检查认证、授权确认、Origin、输入和日志边界。
- [x] 检查敏感文件处理、更新校验和现有回归测试。
- [ ] 未确认任何生产部署已被利用或完全不存在其他漏洞。

## 7. 稳定性分析（Stability Concerns）

- Coverage: Medium
- Inspected evidence: `history/query_limit.rs`、writer、Agent deadlines/reconnect、mount probe、shutdown、前端请求生命周期。
- Exclusions / limits: 未做长时间网络分区、真实阻塞文件系统、断电和全服务压力测试。

重连、写入缓冲、关闭时 drain 和多种错误诊断已有实现。F02–F04 显示“某个动作有超时/并发限制”并不自动意味着整个资源生命周期有界：线程等待结束后线程仍可阻塞，查询限制执行者但不限制等待者，轮询定时器不能结束已经发出的请求。应分别在资源分配点补充约束。

| 发现 | 应验证的恢复结果 |
|---|---|
| F01 | 回滚只恢复本次应用管理的文件。 |
| F02 | 卡死探测不会持续占用新增槽位，已成功的指标继续可用。 |
| F03 | 队列满时快速返回，取消和恢复后计数正确。 |
| F04 | 网络截止后 loading 释放，用户可以重试。 |

- [x] 真实源模块验证查询等待和探测线程行为。
- [x] Rust 连接、deadline、持久化及相关模块测试通过。
- [ ] 未将上述测试替代长期可用性证明。

## 8. 性能分析（Performance Concerns）

- Coverage: Medium
- Inspected evidence: History 查询缓存与限流、批量 writer、Registry 视图、磁盘探测、前端 bundle 和性能文档。
- Exclusions / limits: 未重跑 benchmarks，也未测本次版本在 200/1,000 节点下的 RSS 与 p95。

现有缓存、双重缓存检查、批量持久化和异步推送有明确用途。当前可证实的性能风险集中在 F02、F03 的饱和状态，而非已经证明正常负载吞吐不达标。`docs/benchmarks.md` 有测试条件与历史数值；本报告不把这些数值转换成本次测量，也不把 AGENTS 中的目标当作已实现承诺。

| 项目 | 本次观察 | 结论 |
|---|---|---|
| 入口 JS | 194.62 kB，gzip 71.15 kB | 未见仅凭入口体积就需替换框架的依据。 |
| NodeDetail chunk | 72.51 kB，gzip 21.55 kB | 已拆分路由 chunk；需真实设备测量后再优化。 |
| 查询执行 | 默认 4 并发，缓存前后检查 | 执行有限，等待无限见 F03。 |
| 磁盘探测 | 64 KiB 栈/线程，每挂载点限制 | 总线程量不受固定预算限制，见 F02。 |

- [x] 区分正常路径优化与饱和场景资源风险。
- [x] 记录真实前端产物体积。
- [ ] 未验证历史性能目标或发布二进制内存目标。

## 9. 测试覆盖分析（Testing Gaps）

- Coverage: Medium
- Inspected evidence: Rust 全套结果、Vue 测试、Playwright、脚本夹具、Python 测试、CI 分层。
- Exclusions / limits: 两个 Linux 系统隔离测试忽略；完整 Python、live E2E、跨平台和硬件链路未运行。

测试数量与实际行为断言共同提供支持；本次全部 Rust 默认工作区测试和 Vue 单元测试通过，不能证明报告中的边界已被覆盖。F01 的安装器模拟 manifest 与真实 Rust 返回字段发生漂移，是最明确的测试真实性缺口。其余应增加的回归已写入各问题卡，无需为了数字新建大量同构测试。

| 缺口 | 关联 | 最小新增验证 |
|---|---|---|
| 暂存/最终路径契约 | F01 | 实际 manifest 与恢复文件集合联合测试。 |
| 全局饱和和恢复 | F02、F03 | 超出预算、取消、恢复的行为断言。 |
| 悬置响应 | F04 | 响应头与响应体分别卡住，验证 deadline 和状态复位。 |
| 管理动作审计 | F05 | 实际数据库事件及秘密字段缺失断言。 |
| 开发服务器修补 | F06 | 公告特定请求的拒绝和 audit 复查。 |
| 无认证历史数据 | F07 | 带存量合成审计记录的访问矩阵。 |

- [x] 记录每类实际测试的退出状态与跳过原因。
- [x] 确认仓库存在单独 live E2E runner，CI 有执行入口。
- [ ] 本次未完整执行该 live E2E runner，不能写成“全部端到端通过”。

## 10. 可维护性分析（Maintainability Concerns）

- Coverage: Medium
- Inspected evidence: 670 个文本文件盘点、模块拆分、错误定义、重复 helper 检索、前端组件/store。
- Exclusions / limits: 未计算全量圈复杂度或代码克隆比率。

主要维护成本来自 Shell、Rust、TypeScript、Python 之间的契约与验证环境，不是单一语言风格。现有模块具备较清楚的职责，发现均可通过局部修改解决。盘点中 `nodelite-server/tests/integration/settings_routes.rs` 为 804 行，超过项目 800 行硬限制；这是现有测试文件的规范偏差，可在相关测试修改时按行为拆分。仅凭多出四行不足以再计为独立工程风险。

本维度没有额外独立发现。F01 应用跨语言契约测试降低同步成本；F04 应继续复用统一 API，而不是让每个页面自行实现超时。

- [x] 检索已有工具与共享逻辑，未建议重复实现。
- [x] 检查生产/测试代码语境，未把测试中的断言方式误报为运行时 panic。
- [ ] 未将目录拆分或代码行数当作完整可维护性证明。

## 11. 设计分析（Design / Principles Concerns）

- Coverage: Medium
- Inspected evidence: 状态所有权、manifest 路径、资源分配、取消安全与统一 API。
- Exclusions / limits: 未形式化全部并发状态机，未主张抽象层越多越好。

| 设计问题 | 实际影响 | 建议 |
|---|---|---|
| F01：配置位置混合了两个含义 | 暂存路径扩大恢复范围 | 用两个显式参数或明确的清单请求对象表达。 |
| F02/F03：局部限制未覆盖总预算 | 后台线程/等待者仍可增长 | 把限制放在资源实际产生的位置。 |
| F04：请求生存期不受调用契约约束 | 共享 store 的 finally 可长期无法执行 | API 层统一总截止与取消语义。 |

没有证据要求引入微服务、通用事件总线、全新 ORM 或重写前端。原则映射详见第 30 节。

- [x] 设计意见均关联可观察的失败场景。
- [x] 优先局部可验证的修补。
- [ ] 不据依赖注入层数、文件数或风格作质量保证。

## 12. 发布分析（Release Concerns）

- Coverage: Medium
- Inspected evidence: `.github/workflows/`、安装升级脚本、Web 更新、checksum、来源证明文档、TUF 发布与 updater。
- Exclusions / limits: 未发布任何资产，未变更生产服务，未做真实回滚、完整多平台 release 构建或签名密钥仪式。

发布配置具有分层检查、Action 固定、锁文件与来源校验；升级脚本还考虑停服一致备份、SQLite sidecar、版本/就绪检查和失败恢复。F01 说明恢复机制自身的文件集合必须比“有备份”更严格。F06 的 moderate 公告不会被现有 high 门槛拦住，也需要明确跟踪。

| 发布门槛 | 处理建议 |
|---|---|
| F01 | 下一次包含安装器的稳定发布前修复并验证恢复范围。 |
| F02/F03 | 在多挂载主机/高查询并发部署前补资源预算。 |
| F04 | 与下一次前端稳定发布一起修复并验证重试体验。 |
| F06 | 升级开发工具修复版本并重新生成锁文件。 |
| 运行环境验证 | 在 CI 规定的 Node 和 Python 环境补跑本次受限测试。 |

- [x] 检查真实 Rust manifest 与 Shell 消费端。
- [x] 执行可行的编译和静态发布检查。
- [ ] 未依据本次 debug 构建签发生产发布结论。

## 13. 文档分析（Documentation Analysis）

- Coverage: Medium
- Inspected evidence: README、SECURITY、CONTRIBUTING、CLAUDE、config 示例、docs/development、configuration、benchmarks、升级/签名说明和 E2E README。
- Exclusions / limits: 未逐个运行全部命令、验证所有外部链接或历史性能数字。

文档对共享凭据、内存会话、审计不完整、签名材料分离和生产部署建议有明确说明；F05 是已知能力限制，不能写成文档声称存在但实际没有的完整审计。开发及 E2E 文档正确区分了模拟界面与真实后端链路。少量源码指针仍用拆分前路径，例如 SECURITY 的 `audit.rs`/`handlers/api.rs`；可在维护文档时更新到现有模块，此项作为定位便利性观察，不单列风险。

### 文档摘要（Documentation Summary）

| Subtype | Count | Affected Docs | Recommended Action |
|---|---:|---|---|
| UserDocs | 0 | README、配置说明 | F07 策略确定后同步公开模式说明。 |
| OperatorDocs | 0 | SECURITY、更新/恢复说明 | 保留已知边界，修复 F01 后补路径契约。 |
| DeveloperDocs | 0 | development、E2E README | 继续区分测试层级与工具链要求。 |
| ApiDocs | 0 | 配置和接口说明 | F03/F04 修复后说明超时/拒绝语义。 |
| DecisionRecord | 0 | DESIGN、代码设计注释 | 无独立重大缺失结论。 |
| StaleDocs | 0 | SECURITY 源码指针 | 有轻微定位偏差，不重复计为独立发现。 |

本表 Count 仅统计正式问题卡；“0”不代表每一行文档都已核验。

- [x] 区分已知限制、实测证据与历史基准。
- [x] 确认签名/迁移工具的运维边界有说明。
- [ ] 未验证外部基础设施符合文档前提。

## 14. 可观测性与运维分析（Observability / Operability Analysis）

- Coverage: Medium
- Inspected evidence: metrics routes、readiness、Audit writer、History diagnostics、告警配送指标。
- Exclusions / limits: 未检查真实监控服务器、报警阈值、日志归档和通知送达。

项目区分 liveness、readiness 和详细诊断，能够暴露历史/审计写入失败、队列和降级状态。query limiter 已记录等待数量与耗时，这为 F03 提供定位基础；补限制时应沿用指标。F05 的缺口在于动作根本没有业务审计事件，而非仅日志级别不合适。

### 信号摘要（Signal Summary）

| Subtype | Count | Critical Signals Missing | Recommended Action |
|---|---:|---|---|
| Logging | 1 | F05：敏感管理动作的持久事件 | 记录目标、结果与安全归因字段。 |
| Metrics | 0 | F02/F03 的新预算需要配套饱和指标 | 随修复提供拒绝、跳过和恢复计数。 |
| Tracing | 0 | 未形成独立可证实故障 | 无需为审计引入新的 tracing 平台。 |
| HealthCheck | 0 | 已有公开/内部诊断分离 | 保持诊断字段的访问边界。 |
| Alerting | 0 | 实际送达未验证 | 在部署环境单独演练。 |
| Runbook | 0 | 有部署与恢复说明 | 根据 F01 修复同步验证步骤。 |
| Debuggability | 0 | 日志/指标覆盖主链路 | 保持错误上下文脱敏。 |

- [x] 真实 HTTP 探针确认内部诊断在无认证时返回 401。
- [x] 检查队列饱和/写入退化的信号设计。
- [ ] 未验证线上告警是否已正确配置。

## 15. 配置安全分析（Configuration Safety Analysis）

- Coverage: Medium
- Inspected evidence: `nodelite-proto/src/config/`、defaults、server.example.toml、配置编辑与 upgrade manifest。
- Exclusions / limits: 未读取真实生产配置；未穷举全部配置组合。

非回环监听要求认证、2FA 的必要参数与 HTTPS 校验、示例中的资源参数均有明确实现。F01 并非 TOML 解析失效，而是配置文件位置被用于派生另一份持久文件位置。配置完整性应同时覆盖值与派生路径。F07 则要求明确“允许关闭认证”的数据暴露范围。

### 配置摘要（Configuration Summary）

| Subtype | Count | Affected Keys / Files | Recommended Action |
|---|---:|---|---|
| SchemaValidation | 0 | Raw 配置/校验器 | 保持入口验证和默认值一致。 |
| UnsafeDefault | 0 | loopback/no-auth | 未认定默认公网无认证；F07 另列待确认边界。 |
| EnvironmentSeparation | 1 | F01：暂存配置路径 | 明确读取路径与部署路径。 |
| SecretConfig | 0 | auth、告警凭据与序列化 | 已检范围未发现直接泄露，继续保持脱敏。 |
| FeatureFlag | 0 | remote updater 选择与能力 | 保持本地显式启用边界。 |
| ConfigDocs | 0 | 示例与默认参数 | 修复涉及新预算时补齐 schema、默认、公开字段、示例。 |

- [x] 核对认证缺失和 2FA 非法组合的启动拒绝路径。
- [x] 使用相同配置内容复现不同物理路径导致的 manifest 差异。
- [ ] 未保证任意用户配置都在本次验证组合内。

## 16. 数据完整性分析（Data Integrity Analysis）

- Coverage: Medium
- Inspected evidence: History/Audit SQLite、迁移测试、registry/snapshot、Passkey 原子写、升级备份和 updater journal。
- Exclusions / limits: 未做真实断电、大库迁移和全业务备份恢复演练。

已有事务、原子替换、SQLite sidecar 备份和恢复顺序设计，不应误报“没有回滚”或“Passkey 从不备份”。当前正式问题是 F01 的备份范围错误。历史/审计 best-effort 写入有指标和降级语义，属于明确设计取舍，不能不考虑目标就一律要求阻塞核心采集。

### 完整性摘要（Integrity Summary）

| Subtype | Count | Invariants at Risk | Recommended Action |
|---|---:|---|---|
| TransactionBoundary | 0 | SQLite 与文件替换 | 继续保留失败/迁移测试。 |
| Idempotency | 0 | 更新与状态切换 | F04 超时修复禁止自动重放未知结果的写请求。 |
| ConcurrencyConsistency | 0 | 共享注册状态、凭据与快照 | 未形成独立已证实问题。 |
| MigrationSafety | 0 | schema 初始化及升级 | 本次仅覆盖现有测试与源检查。 |
| InvariantValidation | 0 | 外部消息与输入边界 | 维持 sanitize / typed config 验证。 |
| BackupRestore | 1 | F01：仅操作应用自有文件 | 以最终路径生成集合并测试哨兵不变。 |
| Reconciliation | 0 | journal/恢复状态 | 未验证真实故障期间的最终一致性。 |

- [x] 检查数据库主文件与辅助文件恢复处理。
- [x] 现有迁移、历史与审计测试随 Rust 全套通过。
- [ ] 未把单元测试当作真实备份可恢复性演练。

## 17. 隐私与数据治理分析（Privacy / Data Governance Analysis）

- Coverage: Medium
- Inspected evidence: Audit 数据类型/查询/保留、登录 IP 和 UA、节点元数据、配置脱敏与删除流程。
- Exclusions / limits: 未核实运营主体的数据政策、真实外部 GeoIP/告警服务保留期或法律合规要求。

主要隐私相关数据是登录来源 IP、UA、时间、节点标识和配置中的秘密材料。审计字段有长度限制，默认保留 90 天。日志截断属于体积控制，不等于去标识化；是否公开这些字段必须独立于“仪表盘可公开”判断。F07 的真实返回已确认，产品政策不确定，因此不升级为确定的公网泄露。

### 隐私摘要（Privacy Summary）

| Subtype | Count | Affected Data | Recommended Action |
|---|---:|---|---|
| DataInventory | 0 | IP、UA、节点、凭据 | 本报告列出主要类别，未声称完成组织级数据清单。 |
| Minimization | 0 | 审计文本/details | 无额外明确过量采集结论。 |
| AccessBoundary | 1 | F07：审计登录记录 | 先确定无认证模式的公开范围。 |
| Retention | 0 | Audit/History | 已有保留机制；未检查外部备份副本。 |
| Deletion | 0 | 删除节点及其状态 | 未完成备份/外部日志的端到端删除检查。 |
| Export | 0 | API 数据读取 | 未发现独立导出功能缺陷。 |
| TelemetryPrivacy | 0 | 结构化日志与 metrics | 不输出真实凭据，保持字段脱敏。 |

- [x] 探针仅使用合成用户与文档地址段 IP。
- [x] 区分公开状态与安全审计字段。
- [ ] 不输出组织级隐私或法规符合性结论。

## 18. 可访问性与交互正确性（Accessibility / UX Correctness Analysis）

- Coverage: Medium
- Inspected evidence: `NativeDialog.vue`、Chart/StepUp/Delete 对话框、相关测试、`modal-keyboard.spec.ts`、loading/error 状态。
- Exclusions / limits: 未使用读屏器，未完成全部配色/缩放/响应式断点或 WCAG 合规检查。

原生 dialog 提供模态层，组件补充 Tab 边界、Escape 和关闭时焦点恢复，相关键盘夹具随 Chromium 测试通过。未发现需要另列的键盘阻塞问题。F04 的持续加载属于所有用户都会遇到的交互状态风险，也会使依靠状态反馈的用户难以判断操作是否结束。

### 交互摘要（Accessibility Summary）

| Subtype | Count | Affected Workflows | Recommended Action |
|---|---:|---|---|
| SemanticStructure | 0 | 原生 dialog / 表单 | 保持标签与语义控制。 |
| KeyboardFocus | 0 | 对话框 | 保留焦点恢复和键盘测试。 |
| ResponsiveVisual | 0 | 仪表盘/详情页 | 本次未覆盖所有设备，不能判为全量合格。 |
| ErrorState | 0 | API 错误显示 | 随 F04 增加超时反馈。 |
| LoadingState | 1 | F04：设置加载 | deadline 后恢复可操作状态。 |
| UXStateCorrectness | 0 | 写操作超时 | 不自动重放结果未知的写请求。 |

- [x] 检查原生模态、焦点与 Escape 处理。
- [x] 运行桌面 Chromium 交互夹具。
- [ ] 未给出 WCAG 等级认证。

## 19. 供应链与可复现性（Supply Chain / Reproducibility Analysis）

- Coverage: Medium
- Inspected evidence: Cargo.lock、pnpm-lock、CI/release、Action SHA 检查、安装校验、TUF 信任链、发布来源文档。
- Exclusions / limits: 未下载验证真实发布资产，未接触生产签名密钥；Python 发布工具未完整执行。

Cargo audit 在本次公告库下为 0，不能推出不存在未知漏洞。pnpm 返回的是 6 个 moderate 包结果、5 条独立公告；minimatch 两个解析版本被分别计入结果。以下不是 5 条新增正式发现，统一作为 F06 的依赖证据，并区分直接可见的开发入口与尚未证实的利用面。

| 依赖/锁定版本 | 公告 | 修复范围 | 本项目可达性判断 |
|---|---|---|---|
| Vite 6.4.4 | [GHSA-8g5g-mxqh-grgj](https://github.com/advisories/GHSA-8g5g-mxqh-grgj) | ≥6.4.5 | `pnpm dev` 实际使用；环境是否暴露需部署者确认。 |
| @vitest/mocker 4.1.11 | [GHSA-qxg6-89mg-rqv8](https://github.com/advisories/GHSA-qxg6-89mg-rqv8) | ≥4.1.12 | 当前测试依赖命中；未见项目直接配置相关 mock server 插件，利用路径未确认。 |
| @vitest/mocker 4.1.11 | [GHSA-xp26-3454-mjgp](https://github.com/advisories/GHSA-xp26-3454-mjgp) | ≥4.1.12 | Windows 特定路径风险，本次非 Windows 环境。 |
| glob 13.0.6 | [GHSA-3g3m-2fjr-6pw4](https://github.com/advisories/GHSA-3g3m-2fjr-6pw4) | ≥14.0.2 | Vue test-utils → js-beautify 传递依赖；未复现取消绕过。 |
| minimatch 10.2.5 / 10.2.6 | [GHSA-g3hv-75wg-m249](https://github.com/advisories/GHSA-g3hv-75wg-m249) | ≥11.0.1 | 测试/ESLint 工具链传递依赖；未证实产品接收不可信 glob 输入。 |

### 供应链摘要（Supply Chain Summary）

| Subtype | Count | Affected Surface | Recommended Action |
|---|---:|---|---|
| DependencyProvenance | 1 | F06：开发依赖公告 | 更新直接工具链并跟踪传递依赖。 |
| Reproducibility | 0 | locked/offline 构建、工具链声明 | 当前机器重现编译；尚非逐字节 reproducible build 证明。 |
| CIIntegrity | 0 | 固定 Action 与权限配置 | Action pin 检查通过；未验证 GitHub 组织设置。 |
| ArtifactProvenance | 0 | checksum、来源证明、签名更新 | 保留现有链路；真实资产验证未运行。 |
| RegistryHygiene | 0 | 公开更新代次和角色材料 | 未检查实际远端仓库内容。 |

- [x] 查阅在线公告输出并记录解析版本。
- [x] Rust 构建使用锁文件和离线依赖。
- [ ] 没有将来源校验机制的存在视为实际密钥保管安全证明。

## 20. 成本与资源经济性（Cost / Resource Economics Analysis）

- Coverage: Medium
- Inspected evidence: 线程、查询等待、缓存、批写、审计文本上限、数据保留、构建输出。
- Exclusions / limits: 无真实云账单、磁盘增长曲线和外部服务计费测量。

成本主要来自常驻内存、磁盘历史、线程、查询并发和构建依赖，而不是模型调用。F02/F03 的预算缺口可把故障放大成资源开销。现有缓存上限、保留期及写入队列说明项目已经在主动约束资源，建议补齐缺口而非重做全部运行时。

### 资源摘要（Cost Summary）

| Subtype | Count | Cost Driver | Recommended Action |
|---|---:|---|---|
| UnboundedWork | 2 | F02 线程、F03 等待者 | 在分配点设置预算和拒绝策略。 |
| ExternalApiCost | 0 | GeoIP、告警发送 | 代码可见用途明确，实际费用未核实。 |
| LLMCost | 0 | 不适用 | 不增加模型配额系统。 |
| InfrastructureSizing | 0 | Server/Agent | 用目标部署负载实测后定容量。 |
| ObservabilityCost | 0 | Audit/History | 保留现有长度、队列和保留约束。 |
| CostVisibility | 0 | 运行指标 | 新预算加入饱和指标，不另建计费平台。 |

- [x] 记录本次独立构建缓存并在验证后清理。
- [x] 不把历史文档 RSS 当作所有规模的资源上限。
- [ ] 未提出没有账单证据的降本百分比。

## 21. AI / LLM 安全（AI / LLM Safety Analysis）

- Coverage: Not assessed
- Inspected evidence: workspace 四 crate、前端依赖及运行入口，未见产品模型调用、检索增强或模型工具执行链路。
- Exclusions / limits: 开发者 Agent 配置和历史 worktree 不属于本次产品运行时安全面。

本维度不适用，不计入单独分数；不把监控 Agent 的远程更新错误地当作 LLM tool authorization。

### AI 安全摘要（AI Safety Summary）

| Subtype | Count | Boundary Crossed | Recommended Action |
|---|---:|---|---|
| PromptInjection / ToolAuthorization | 0 | 不适用 | 无产品模型工具链。 |
| RAGLeakage / OutputValidation | 0 | 不适用 | 无产品检索与生成输出。 |
| ModelFallback / EvalGap / AbuseCost | 0 | 不适用 | 不凭开发配置制造产品 AI 风险。 |

- [x] 先检查适用性，再标记 Not assessed。
- [ ] 未评估本项目之外的 AI 开发工具供应商。

## 22. 降级与防御性代码（Fallback / Defensive Code Analysis）

- Coverage: Medium
- Inspected evidence: `unwrap_or` / catch / ignored result 扫描、mount probe、History/Audit 降级、前端 ApiAbortError 和 realtime 回退。
- Exclusions / limits: 未逐条触发所有降级分支。

多数已检降级具有上下文：磁盘超时复用最后成功值并告警；审计/历史丢弃有计数；API 在认证导航开始后用 ApiAbortError 阻止后续链条；localStorage 不可用时仍执行登出导航。把这些分支一律改成 panic 或强制阻断会降低可用性。

F02 的问题是资源总量，不是缓存降级本身；F04 是未规定完成时间，不是吞掉了一个已经产生的错误。本次没有额外确认的静默回退问题。

### 降级摘要（Fallback Summary）

| Subtype | Count | KeepWithAlert | FailFast | Remove |
|---|---:|---:|---:|---:|
| SilentFallback | 0 | 0 | 0 | 0 |
| EmptyCatch | 0 | 0 | 0 | 0 |
| CompatibilityBranch | 0 | 0 | 0 | 0 |
| SilentCorrection | 0 | 0 | 0 | 0 |
| DefensiveGuess | 0 | 0 | 0 | 0 |

上表统计正式问题卡，不统计已接受的降级分支数量。

- [x] 搜索结果经过生产/测试及行为语境筛选。
- [x] 区分无意丢错与有指标的 best-effort 策略。
- [ ] 未保证所有未触发异常路径都符合预期。

## 23. 测试真实性（Testing Authenticity Analysis）

- Coverage: Medium
- Inspected evidence: Rust 实际执行日志、安装 manifest fixture、Vue 单元测试、Playwright mocked/live 分层和定向探针。
- Exclusions / limits: 未做完整变异测试，也未运行全部跨系统测试。

### 信心评估（Confidence Assessment）

| Test Area | Real Confidence | Risk | Action |
|---|---|---|---|
| Rust 认证/历史/审计/更新模块 | High（对实际执行的行为） | 不覆盖生产部署、真实签名仪式或所有平台差异 | Keep；保留实际错误、恢复与数据库断言。 |
| Agent deadlines/reconnect/token 集成 | High（对本次测试环境） | 不覆盖真实 NFS 和 systemd 流控 | Keep；补 F02 全局饱和。 |
| 安装器模拟 manifest | Medium | 漏掉真实 Passkey 字段，F01 可逃逸 | Rewrite 局部夹具契约；保留已有有价值用例。 |
| Vue 单元与 store 测试 | Medium | Promise 一直不结束的网络行为可能未覆盖 | Keep；补 F04。 |
| Chromium route-mocked UI | Medium | 前端行为可验证，Rust 认证/WS 集成错误可逃逸 | Keep；与 live 套件分开解释。 |
| 仓库 live E2E | 本次未运行 | 已有真实 Server/Agent 流程与 CI 入口，但本次不能确认运行结果 | Keep；在指定工具链复跑。 |
| Python 发布/迁移 | Low（本次执行受阻） | 缺依赖导致导入失败，签名流程验证不完整 | 配齐仓库声明环境后重跑，不能删除测试。 |
| 系统流控隔离测试 | 本次未运行 | 2 项 ignored 需要 network namespace/systemd | 使用原隔离 harness 验证。 |

### 有价值的测试（Valuable Tests）

真实数据库迁移/写入、认证撤销与会话容量、重连 deadline、凭据持久化及 updater 激活恢复，比仅断言 helper 被调用更能保护行为。本次探针直接加载实际模块或二进制，没有拷贝逻辑重新实现后自证。

### 需要谨慎解释的测试（Suspicious Tests）

明确存在的问题是 F01 模拟清单缺字段；不能据此否定全部安装器测试。浏览器路由 mock 是合理测试分层，问题只在将其过度解释为完整后端通过。测试日志中的代理连接拒绝也需要按被测流程判断，不能抹去或假装完全没有后端依赖。

### 缺失验证（Missing Tests）

优先补各 Finding 的回归，再补本次环境受限的现成测试。不要把缺少新测试与现有测试没有运行混为同一问题。

- [x] 逐类给出实测信心和逃逸风险。
- [x] 统计不包含 skipped/ignored 的通过数。
- [ ] 未进行“全部测试都是真实有效”的无条件背书。

## 24. 类型安全分析（Type Safety Analysis）

- Coverage: Medium
- Inspected evidence: Rust unsafe/SAFETY、错误类型、serde 边界、`sanitize.rs`、TS typecheck、API 泛型。
- Exclusions / limits: 未运行 Miri、fuzzing 或 Windows/macOS ABI 测试。

unsafe 主要处于操作系统接口边界，抽查的 `statvfs` 初始化与 FD 锁使用有具体安全条件说明。TS 的 `res.json() as Promise<T>` 提供编译期调用契约，不能充当运行时 JSON schema 校验；但当前没有证据证明某个返回形状导致实际缺陷，因此不单凭类型断言开问题卡。

### 类型安全摘要（Summary）

| Subtype | Count | Critical | High | Medium | Low |
|---|---:|---:|---:|---:|---:|
| UnsafeBlock | 0 | 0 | 0 | 0 | 0 |
| TypeAssertion | 0 | 0 | 0 | 0 | 0 |
| InputBoundary | 0 | 0 | 0 | 0 | 0 |
| OutputLeak | 0 | 0 | 0 | 0 | 0 |
| BooleanTrap | 0 | 0 | 0 | 0 | 0 |
| StringlyTyped | 0 | 0 | 0 | 0 | 0 |
| ErrorType | 0 | 0 | 0 | 0 | 0 |

未形成独立类型安全发现，表中不计已经按隐私分类的 F07，也不表示已经证明全部 unsafe 正确。

- [x] Rust Clippy 和前端类型检查通过。
- [x] 区分编译类型与运行时输入验证。
- [ ] 未声称通过形式化内存安全验证。

## 25. 前端状态分析（Frontend State Analysis）

- Coverage: Medium
- Inspected evidence: API client、Pinia stores、usePolling、useRealtimeData、请求取消及主要页面/对话框测试。
- Exclusions / limits: 未进行所有路由多窗口、真实弱网和快速切换组合测试。

共享资源由 Pinia 管理，轮询生命周期留在组合函数，实时更新由专门逻辑处理，总体职责合理。F04 是共享 store 的准入与请求生命周期之间的实际耦合问题；修复应位于统一 API 层，并在 store 层验证恢复，而不是复制多份定时器。

### 前端摘要（Summary）

| Subtype | Count | Affected Components |
|---|---:|---|
| ComponentSize | 0 | 未凭组件行数确定独立缺陷。 |
| StateDuplication | 0 | 已检 stores 与服务端副本职责合理。 |
| PropDrilling | 0 | 无独立发现。 |
| EffectChain | 0 | 未形成可重现的循环副作用。 |
| UIBusinessCoupling | 0 | 无需重做现有分层。 |
| DOMasState | 0 | 焦点和 dialog 的 DOM 操作具有交互职责。 |
| RequestState | 1 | F04：api/client、settings store、轮询。 |
| RenderPerf | 0 | 未用真实设备定量测试。 |

- [x] 检查加载、错误、取消和下一次刷新之间的关系。
- [x] 运行单元与浏览器夹具。
- [ ] 未确认所有实时与 REST 混合竞争场景都已覆盖。

## 26. 后端 API 分析（Backend API Analysis）

- Coverage: Medium
- Inspected evidence: `startup.rs` 路由、auth middleware、settings guards、api_routes、History 查询和审计 API。
- Exclusions / limits: 未为所有端点执行完整黑盒契约矩阵。

写请求有 body 限制和敏感操作确认；审计查询验证时间范围并限制返回条数；历史查询具备缓存和执行并发限制。缺口是 F03 的过载拒绝策略与 F07 的可选认证数据边界。泛泛统一所有错误文案的价值低于这些行为契约。

### API 摘要（Summary）

| Subtype | Count | Affected Endpoints |
|---|---:|---|
| ApiConsistency | 0 | 暂无独立一致性问题。 |
| Validation | 0 | 输入、时间与 body 验证已检。 |
| Auth | 1 | F07：`/api/audit-log`，风险认定待确认。 |
| NplusOne | 0 | 未形成已证实 N+1 查询问题。 |
| Caching | 0 | 不同键可绕过复用但本身符合语义；资源等待见 F03。 |
| ErrorResponse | 0 | F03 修复需定义饱和状态码。 |
| BusinessLogic | 0 | 无独立发现。 |
| DataFlow | 1 | F03：历史查询的等待准入。 |

- [x] 真实 HTTP 验证审计与诊断访问差异。
- [x] 阅读查询限制与其 HTTP 装配边界。
- [ ] 未把失败认证限流当作已认证用户的全请求限流。

## 27. 依赖重量分析（Dependency Weight Analysis）

- Coverage: Medium
- Inspected evidence: 四 crate manifest、Cargo.lock、web package/lock、生产 bundle、构建日志。
- Exclusions / limits: 未运行 cargo-bloat 或逐库 tree-shaking 分析，未测 release 二进制分项体积。

依赖数量不等于运行时重量。Cargo.lock 363 项包含跨平台/构建依赖；前端 audit 元数据统计 303 项也不能直接解释成 303 个浏览器运行库。正式 findings 中依赖问题是 F06 的公告，不是某库看起来“大”。

### 依赖看板（Dependency Scoreboard）

| Dependency | Status | Weight | Transitives | Used For | Recommended Action |
|---|---|---|---|---|---|
| tokio / axum | Healthy | 未单独归因 | 未按单库统计 | 异步 Server、HTTP/WS | 保留，已有明确职责。 |
| rusqlite（bundled） | Healthy | 编译 SQLite，有体积成本 | 未按单库统计 | 历史与审计持久化 | 保留；没有改 ORM 的证据。 |
| webauthn-rs / vendored OpenSSL | Healthy（重量未完整评估） | 原生编译成本可见 | 未按单库统计 | Passkey 与相关加密依赖 | 保留功能边界，只有实测再决定裁剪。 |
| Vue / Router / Pinia / i18n | Healthy | 共享入口合计 194.62 kB，gzip 71.15 kB；非各库独立体积 | 未按单库统计 | UI、路由、状态与语言 | 保留已实现的路由拆包。 |
| Vite / Vitest | 功能所需；安全更新待办 | 开发期依赖，不全部进入生产包 | 见 F06 | 构建和测试 | 升级修复版本，不建议删除测试工具。 |
| Python TUF / cryptography | 功能所需；本次执行不完整 | 发布工具环境，非 Agent 常驻依赖 | 未测 | 签名、迁移和运营工具 | 使用声明的隔离环境补验证。 |

- [x] 区分开发、构建、跨平台与运行时依赖。
- [x] 记录实际 bundle，而非按包数量宣称过重。
- [ ] 未给出未经测量的可裁剪比例。

## 28. 代码一致性（Code Consistency）

- Coverage: Medium
- Inspected evidence: rustfmt、Clippy、ESLint，错误返回、导入、命名和共享 helper 抽查。
- Exclusions / limits: 未逐行比较所有模块的命名与语体。

配置好的格式和 lint 检查均通过，说明机器约定在本次环境下稳定。存在中英文注释并行、部分公共错误采用手工实现等差异，但未发现由此造成的独立行为问题。单独为了统一表面形式大面积改动会增加审查噪声，本次不建议这样做。

本维度无独立问题卡；804 行测试文件的规范偏差见可维护性章节。

- [x] 使用项目现有检查入口。
- [x] 没有执行自动格式化写回或改动应用源码。
- [ ] 通过 lint 不代表业务语义一致性已经完备。

## 29. 注释与说明覆盖（Comment Coverage）

- Coverage: Medium
- Inspected evidence: 模块顶层说明、unsafe 安全条件、持久化和取消说明、API 生命周期注释、SECURITY 的能力边界。
- Exclusions / limits: 未计算注释率，未逐条验证全部历史注释。

关键注释通常解释“为什么”，例如恢复顺序、阻塞探测 pending、防止签名路径被替换、错误脱敏和会话重放。F02 相关注释正确说明共享 deadline 限制的是一轮等待，但不能推导为全局线程有界；报告保留这一语义差别。通用 API 顶部注释列出了认证与返回内容处理，却未承诺 timeout，本次也没有伪造过期注释问题。

本维度无独立问题卡。优先在 F01、F03、F04 修复时补充路径、排队和取消语义，而非提高注释百分比。

- [x] 抽查关键边界的设计说明与实际实现。
- [x] 区分责任说明、未承诺的能力与错误注释。
- [ ] 未证明每个公共接口都具备完善文档。

## 30. 工程原则遵循（Principles Compliance）

### 存在缺口的原则（Principles Violated）

| Principle | Violations | Severity | Affected Areas |
|---|---:|---|---|
| 4.6 最小权限 / 文件操作范围 | 1 | Medium | F01：临时派生路径扩大安装器恢复集合。 |
| 10.2 资源必须有界 | 2 | Medium | F02 的线程、F03 的等待队列。 |
| 10.4 外部调用超时 | 1 | Medium | F04 的默认 fetch 与响应体总时限。 |
| 8.1 测试可观察行为 / 边界契约 | 1（F01 的同一根因链） | Medium | 模拟 manifest 未反映真实 Passkey 路径，需交叉验证。 |

原则计数不是新问题数；严重度按实际场景调整，没有直接套用原则文档中的默认高危级别。F05 属于审计能力缺口，F06 是公告修补，F07 是待确认访问策略，不强行归为 DRY、SRP 或 Fail-Fast 违规。存在相同格式、直接调用数据库、固定 enum 分支，并不足以自行证明架构设计错误。

### 已做好的原则（Principles Respected）

- 核心配置与协议集中复用；职责在 crate 和 Server 子模块间较清楚。
- 原子写入、数据库事务、会话容量、写入队列及取消时 RAII 释放有明确设计。
- 外部输入和秘密输出的边界校验并非仅依赖前端。
- 更新器区分下载、信任验证和特权激活；本地启用与签名信任未交给远程请求自行决定。
- 测试中有真实数据库、连接、失败和恢复场景；适合保留并补边界，而不是推翻全部测试。

## 31. 建议修复顺序（Recommended Fix Order）

### 尽快处理（Fix Immediately）

没有已证实正在发生的 Critical/High 事故，不建议未经验证就紧急部署。若即将执行安装器升级，先修复 F01；若团队正在对外共享 Vite 开发端口，优先修复 F06。F07 先明确无认证公开模式的政策，采用该部署方式时同步复核外层访问控制。

| 顺序 | 事项 | 验收点 | 估算 |
|---:|---|---|---|
| 1 | F01：最终路径语义 | 暂存目录哨兵不进入备份/回滚集合；真实 manifest 测试通过 | 2–4 小时 |
| 2 | F06：开发工具公告 | 已解析到修复版本，目标公告消失，测试/构建通过 | 1–2 小时起 |

### 下一稳定版前处理（Fix Before Stable Release）

| 顺序 | 事项 | 验收点 | 估算 |
|---:|---|---|---|
| 3 | F02：全局探测预算 | 多路径阻塞与挂载变化均不突破并发上限 | 0.5–1 天 |
| 4 | F03：有限队列与等待 | 满额快速拒绝、超时/取消释放、恢复正常 | 约 0.5 天 |
| 5 | F04：API 总截止时间 | 响应头/body 悬置都能退出，loading 复位，写请求不自动重放 | 2–4 小时 |
| 6 | F05：关键管理动作审计 | 成功/拒绝/持久化失败可区分，秘密不落库 | 0.5–1 天 |
| 7 | F07：按确定策略补边界 | 带存量审计数据的访问矩阵满足明确政策 | 2–4 小时 |

### 后续安排（Schedule Later）

先补齐现成系统隔离、Python 发布工具和 live E2E 的执行证据，再按目标部署规模做查询拥塞/历史积压与多挂载测试。更完整的读屏器测试、逐库体积归因和真实恢复演练属于补充验证，不是在本报告之外凭空增加缺陷。

### 暂不投入（Ignore for Now）

不因文件数量改成微服务，不因依赖数量替换框架，不为注释率或命名审美批量重写；未证实可达的传递依赖公告继续跟踪，不夸大成生产远程代码执行。没有需要长期结构性重写的证据，因此不列全仓重构计划。

## 32. 低成本优先项（Quick Wins）

| 动作 | 直接收益 | 注意事项 |
|---|---|---|
| 更新 Vite/Vitest 的兼容修复版本 | 降低已知开发工具风险 | 更改前检查 override 和 lock；不要跳过测试。 |
| 为真实 manifest 加入暂存/最终路径断言 | 用小契约测试锁住 F01 | 测试必须消费真实返回集合，不能再次复制假清单。 |
| 为 API 增加可复用的 deadline 测试夹具 | 防止 F04 修复只覆盖 fetch 而漏 body | 不自动重试未知结果的写操作。 |
| 在正确 Node/Python 环境复跑现有受限测试 | 补充可靠验证证据 | 本次未安装或修改用户工具链。 |

## 33. 证据、复现与交付记录

本地证据目录：[output/audit-2026-11-07](/Users/bernard/Code/NodeLite/output/audit-2026-11-07)。该目录被 Git 忽略，不随报告提交；包括盘点、检查日志、公告 JSON、探针源码/输出和本次源归档。报告中的摘要可独立阅读，原始材料用于复核。

| 证据文件 | 内容 |
|---|---|
| `inventory.tsv` | 第一方跟踪文本文件与行数。 |
| `cargo-test.log`、`clippy.log`、`build-server.log` | 云端实际编译与验证日志。 |
| `vitest.log`、`typecheck.log`、`eslint.log`、`web-build.log`、`playwright.log` | 前端检查和浏览器夹具结果。 |
| `cargo-audit.json`、`pnpm-audit.json` | 本次公告数据库的实际匹配结果。 |
| `python-tests.log`、`script-tests.log`、`static-checks.log` | 环境缺依赖导致的验证限制、已完成部分及静态脚本检查。 |
| `probes/main.rs`、`probes/Cargo.toml`、`probe-rust.log` | F02/F03 的实际源模块资源探针。 |
| `probe-api.mjs`、`probe-api.log` | F04 的实际 API 模块调用探针。 |
| `probe-server.py`、`probe-server.log` | F01/F07 的真实二进制 manifest 与匿名审计观察。 |
| `cleanup.log` | 本次独立云端 target 清理结果。 |

Rust 资源探针的相对源码引用基于云端目录布局；将 `probes/` 放在源树根目录下运行，不能直接在本地 `output/` 布局下使用相对引用。复现命令如下，均应在本次隔离源码副本中执行：

```sh
# /home/compiler/NodeLite/audit-20261107-ece3885
CARGO_BUILD_JOBS=2 cargo run --manifest-path probes/Cargo.toml --offline
NODELITE_SKIP_WEB_BUILD=1 CARGO_BUILD_JOBS=2 cargo build --locked --offline -p nodelite-server
python3 probe-server.py
```

HTTP 探针在首次运行创建私有 fixture 目录，重复运行应使用新的任务副本/fixture 路径，不要删除或覆盖原证据。实际观察摘要：

```text
实际配置清单：probe-fixture/config/passkeys.json
暂存配置清单：probe-fixture/staging/passkeys.json
anonymous_audit: status=200, fixture_event_visible=true
anonymous_diagnostics_status=401
query_limiter: permits_in_use=1, waiting=32, limit=1
mounts=32, simultaneously_blocked_probe_threads=32, unavailable_results=32
API: suppliedAbortSignal=false, pendingAfter100ms=true
```

探针不连接生产、不读取真实审计或密钥；阻塞线程使用可释放同步原语并正常收尾，服务进程在 finally 中终止。临时服务 debug 二进制 SHA256 为 `93e261821983879234ee59276f368211bbcf1717e1eb4f660b487af599652e97`，仅用于标识本次验证产物，不是 release 产物。

本次由 GPT 主 Agent 独立完成，**Worker 调用 0，创建 Worktree 0，清理 Worktree 0**。未修改应用源码、测试、配置、锁文件或依赖；生成报告及被忽略的验证材料。云端使用已有 `~/NodeLite` 内唯一新目录 `/home/compiler/NodeLite/audit-20261107-ece3885`，未覆盖其原有项目文件；验证完成后仅通过指向该目录的 `cargo clean --target-dir` 清理两个自有构建目录，工具报告移除约 8.2 GiB 与 129.1 MiB，可复查源码、合成 fixture 与日志约 11 MiB 保留。未清理用户已有工作树或共享缓存。

Git 交付只包含本报告；用户已有 `NodeLite-architecture-report.html` 保留为未跟踪。没有推送、PR、Issue、部署或业务修复操作。报告按技能的 full 模式格式检查；具体报告提交号在交付回复中提供。
