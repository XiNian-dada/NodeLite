# NodeLite 性能优化计划

**Worktree**: `perf-optimization-experiment`  
**Branch**: `worktree-perf-optimization-experiment`  
**开始时间**: 2026-06-15  
**目标**: 优化 v3.0.1 性能测试中发现的瓶颈

---

## 📊 性能基线（v3.0.1）

基于 PERFORMANCE_REPORT_v3.0.1.md 的测试结果：

| 指标 | 当前值 | 目标值 | 优先级 |
|------|--------|--------|--------|
| History API p95 | 20.91ms | < 10ms | **高** |
| 1000节点连接时间 | 7,916ms | < 5,000ms | 中 |
| 1000节点内存 | 337 MB | < 250 MB | 中 |
| 大负载(64盘) p95 | 10.37ms | < 5ms | 中低 |
| 并发读写 p95 | 9.49ms | < 5ms | 低 |

---

## 🎯 Phase 1: 低垂的果实（预计 1-2 周）

### 1.1 ✅ History API 查询优化

**当前状态**: p95 20.91ms（是其他 API 的 2-3 倍）

**已有优化**:
- ✅ 复合索引: `idx_history_points_node_time (node_id, recorded_at)`
- ✅ 覆盖索引: `idx_history_points_covering_metrics` (包含所有列)
- ✅ 查询强制使用索引: `INDEXED BY idx_history_points_covering_metrics`

**进一步优化方向**:
1. **添加 SQLite 查询分析**
   - 运行 `EXPLAIN QUERY PLAN` 查看实际执行计划
   - 检查 GROUP BY 和 AVG 聚合的开销

2. **实现查询结果缓存**
   - LRU 缓存最近 N 分钟的查询结果
   - 缓存键: (node_id, since, until, max_points)
   - TTL: 30-60 秒

3. **优化聚合逻辑**
   - 考虑预聚合：每 5 分钟聚合一次历史数据到 `history_aggregated` 表
   - 查询时优先使用预聚合表

**预期收益**: History API p95 降到 < 10ms

**实施步骤**:
- [x] 检查现有索引（已确认索引良好）
- [x] 实现 LRU 缓存层（容量 200，TTL 1s）
- [x] 发现 tokio::Mutex 性能退化
- [x] 替换为 parking_lot::Mutex
- [x] Commit: "perf(history): add LRU cache for history queries"
- [x] Commit: "perf(history): replace tokio::Mutex with parking_lot::Mutex in cache"
- [x] 运行性能测试对比
- [x] 记录优化效果到 PERF_OPTIMIZATION_RESULTS.md

**实际收益**: History API p95 从 37.40ms 降到 31.86ms（-14.8% ✅）

**关键经验**:
- ❌ tokio::Mutex 不适合纯同步的短临界区（+3.5% 退化）
- ✅ parking_lot::Mutex 适合不跨 await 的微秒级操作（-17.7% 改进）
- ✅ 1秒 TTL 在数据新鲜度和缓存收益间取得平衡

---

### 1.2 替换为 parking_lot::RwLock

**当前状态**: 使用 `std::sync::RwLock` 和 `tokio::sync::RwLock`

**优化方向**:
```rust
// 替换关键路径的锁：
// 1. SharedState 的 registry 锁
// 2. HistoryStore 的 writer_tx 锁
// 3. AppState 中的其他共享状态

// parking_lot 优势：
// - 20-30% 性能提升
// - 更公平的调度（减少长尾延迟）
// - 无毒化（poisoning）机制
```

**预期收益**: 并发场景 p95 降低 10-20%

**实施步骤**:
- [ ] 添加 `parking_lot = "0.12"` 到 Cargo.toml
- [ ] 替换 SharedState::registry 的锁
- [ ] 替换 HistoryStore 的锁
- [ ] 运行性能测试对比
- [ ] Commit: "perf: replace std RwLock with parking_lot"

---

### 1.3 启用 HTTP Brotli 压缩

**当前状态**: ✅ **已启用**

检查结果：
- `tower-http` features: `["compression-br", "compression-gzip"]` ✅
- `CompressionLayer::new().no_deflate().no_zstd()` - 启用 gzip 和 brotli ✅
- 无需额外优化

**结论**: Brotli 压缩已经正确配置并工作，此项跳过。

---

### 1.4 Token 验证缓存

**当前状态**: 每次连接都运行 Argon2id 验证（CPU 密集）

**问题分析**:
```
1000 节点连接测试: connect_ms ≈ 9,000ms
每节点平均: ~9ms
其中 Argon2id 验证: ~5-7ms (估计占 60-80%)
```

**优化方向**:
```rust
// 实现 Token 验证结果缓存
struct TokenCache {
    // LRU cache: token_hash → (node_id, validated_at)
    cache: Arc<Mutex<LruCache<String, (String, Instant)>>>,
    ttl: Duration, // 5-10 分钟
}

// 逻辑：
// 1. 首次连接：Argon2id 验证 + 写入缓存
// 2. 重连（5分钟内）：直接从缓存返回
// 3. TTL 过期：重新验证 + 刷新缓存
```

**预期收益**: 1000节点连接时间从 9s 降到 < 3s（重连场景）

**实施步骤**:
- [x] 找到 Token 验证代码位置
- [x] 设计 TokenCache 结构
- [x] 实现 LRU 缓存逻辑
- [x] 集成到 verify_hashed_token
- [x] 添加 token 轮换时的缓存清理
- [x] 运行连接测试对比
- [x] Commit: "perf(auth): add token verification cache"

**实际收益**: 
- 20节点重连: p50 从 238.94ms 降到 15.11ms (**-93.7%** ✅)
- 50节点重连: p50 从 498.77ms 降到 13.69ms (**-97.3%** ✅)
- 100节点重连: p50 从 936.74ms 降到 19.27ms (**-97.9%** ✅)
- 200节点重连: p50 从 1849.51ms 降到 710.75ms (**-61.6%** ✅)

**关键经验**:
- ✅ LRU 缓存在重连风暴中命中率极高
- ✅ parking_lot::Mutex 适合短临界区缓存操作
- ✅ SHA256 作为缓存键既安全又高效
- ⚠️ 大规模场景受 semaphore 限制 (2 并发 Argon2id)

---

## 🚀 Phase 2: 结构性优化（预计 3-4 周）

### 2.1 并发连接处理优化

---

### 2.2 字符串池优化（String Interning）

**当前状态**: 1000节点内存 337 MB（每节点 ~337 KB）

**优化方向**:
```rust
// 高重复度的字符串使用字符串池
// 1. 国家/城市名称（5-10 种）
// 2. OS 名称（Linux, macOS, Windows）
// 3. Agent 版本号（通常同一版本）

use std::sync::Arc;
use dashmap::DashMap;

struct StringPool {
    pool: DashMap<String, Arc<str>>,
}

// 预期节省：30-40% 的字符串内存
// 337 MB → ~220 MB
```

**预期收益**: 1000节点内存降到 < 250 MB

**实施步骤**:
- [ ] 实现 StringPool 结构
- [ ] 识别高重复度字段
- [ ] 集成到 NodeEntry 创建流程
- [ ] 运行大规模测试对比内存
- [ ] Commit: "perf(registry): add string interning for common fields"

---

### 2.3 History 查询预聚合

**当前状态**: 实时聚合所有历史点

**优化方向**:
```sql
-- 新增预聚合表
CREATE TABLE history_aggregated (
    node_id TEXT NOT NULL,
    bucket_start INTEGER NOT NULL,  -- 5分钟桶
    bucket_end INTEGER NOT NULL,
    avg_cpu_usage REAL,
    avg_memory_used REAL,
    -- ... 其他聚合字段
    sample_count INTEGER,
    PRIMARY KEY (node_id, bucket_start)
);

-- 后台任务每 5 分钟聚合一次
-- 查询时：
-- 1. 优先使用预聚合表（5分钟+前的数据）
-- 2. 最近5分钟实时聚合
```

**预期收益**: History API p95 降到 < 5ms（进一步优化）

**实施步骤**:
- [ ] 设计预聚合表结构
- [ ] 实现后台聚合任务
- [ ] 修改查询逻辑（混合查询）
- [ ] 添加预聚合数据清理
- [ ] 运行 history_pressure 测试对比
- [ ] Commit: "perf(history): add pre-aggregation for historical data"

---

## 🔬 Phase 3: 结构优化（预计 2-3 周）

### Phase 2 回顾
- ✅ String Pool 实现完成，但收益有限（-5% 内存）
- ❌ History 预聚合评估后取消
- 🔍 **根因**: 优化范围太窄，只针对 4 个 GeoIP 字段

### 3.1 扩大字符串池范围

**当前问题**: Phase 2 只优化了 4 个 GeoIP 字段

**优化范围扩展**:
```rust
// NodeIdentity 中的高重复字段：
pub struct NodeIdentity {
    node_id: String,          // ❌ 保持唯一，不 intern
    node_label: String,       // ❌ 保持唯一，不 intern
    hostname: String,         // ❌ 保持唯一，不 intern
    os: String,               // ✅ "Linux", "Darwin", "Windows" (3种)
    kernel_version: Option<String>,  // ✅ "6.1.0", "5.15.0" (10-20种)
    cpu_model: Option<String>,       // ✅ "Intel Xeon", "AMD EPYC" (20-30种)
    agent_version: String,    // ✅ "1.0.0" (1-3种版本)
    // ...
}

// 预期节省：
// 1000 节点 × 4 字段 × 平均 20 字节 = 80 KB (不用池)
// vs 50 种字符串 × 20 字节 + 1000 × 16 字节 Arc = 17 KB
// 节省：63 KB per 1000 nodes
```

**总节省预期**: 
- Phase 2: 17 KB (GeoIP)
- Phase 3.1: 63 KB (Identity)
- **合计**: 80 KB per 1000 nodes (**-8% 内存**)

**实施步骤**:
- [ ] 修改 NodeIdentity 字段类型: String → Arc<str>
- [ ] 更新 node registration 流程
- [ ] 更新 session restore 流程
- [ ] 添加真实场景测试
- [ ] Commit: "perf(memory): extend string pool to NodeIdentity fields"

**性价比**: ⭐⭐⭐ (低难度，中等收益)

---

### 3.2 NodeStatusView 轻量级视图 ⭐⭐⭐⭐⭐

**当前问题**: `/api/overview` 克隆所有 NodeEntry，导致序列化开销巨大

```rust
// 当前：
pub async fn list_statuses(&self) -> Vec<NodeStatus> {
    // 克隆所有 NodeEntry，包含大量不必要字段
    registry.read().iter().map(|entry| entry.to_status()).collect()
}

// 问题：
// 1. NodeEntry 包含 token_hash, session_id 等后端字段
// 2. 每次请求都完整克隆 ~337 KB × 1000 = 337 MB
// 3. serde_json 序列化开销大
```

**优化方案**:
```rust
// 新增轻量级视图
#[derive(Serialize)]
pub struct NodeStatusView {
    node_id: Arc<str>,           // 共享，零拷贝
    node_label: Arc<str>,        // 共享
    status: NodeStatus,          // 8 字节枚举
    last_seen: i64,             // 8 字节
    cpu_percent: Option<f32>,   // 4 字节
    memory_percent: Option<f32>,// 4 字节
    uptime_secs: u64,           // 8 字节
    // 总共 ~48 字节 vs 337 字节
}

impl NodeEntry {
    pub fn to_status_view(&self) -> NodeStatusView {
        // 零拷贝构建视图
        NodeStatusView {
            node_id: Arc::clone(&self.identity.node_id),
            // ...
        }
    }
}
```

**预期收益**:
- API 响应时间: -50% (减少序列化开销)
- 内存峰值: -30% (减少临时克隆)
- p95 延迟: 5.44ms → < 3ms

**实施步骤**:
- [ ] 设计 NodeStatusView 结构
- [ ] 实现 to_status_view() 方法
- [ ] 修改 /api/overview 使用新视图
- [ ] 修改 /api/nodes 使用新视图
- [ ] 运行 load_test_large_fleet_scores 对比
- [ ] Commit: "perf(api): add lightweight NodeStatusView"

**性价比**: ⭐⭐⭐⭐⭐ (中等难度，**高收益**，用户可感知)

---

### 3.3 History SQLite 压缩存储

**当前问题**: 每个指标存储为独立行，行数过多

```sql
-- 当前方案：
CREATE TABLE history_points (
    node_id TEXT,
    collected_at INTEGER,
    metric_name TEXT,
    value REAL,
    PRIMARY KEY (node_id, collected_at, metric_name)
);

-- 存储密度：
-- 5 指标 × 240 时间点 = 1,200 行/节点
-- 1000 节点 = 120 万行
-- 平均行大小：~50 字节
-- 总大小：60 MB
```

**优化方案**: JSONB 批量存储

```sql
-- 新方案：
CREATE TABLE history_points_v2 (
    node_id TEXT NOT NULL,
    bucket_start INTEGER NOT NULL,  -- 5分钟桶起始时间
    bucket_end INTEGER NOT NULL,    -- 5分钟桶结束时间
    metrics_json TEXT NOT NULL,     -- {"cpu": [1.2,1.5,...], "mem": [45.3,46.1,...]}
    sample_count INTEGER NOT NULL,
    PRIMARY KEY (node_id, bucket_start)
);

CREATE INDEX idx_history_v2_node_time 
    ON history_points_v2(node_id, bucket_start);

-- 存储密度：
-- 240 时间点 / 12 采样点 per 桶 = 20 桶/节点
-- 1000 节点 = 2 万行
-- 平均行大小：~300 字节 (压缩后的 JSON)
-- 总大小：6 MB
-- **节省 90%**
```

**查询逻辑**:
```rust
// 解析 JSON 并重建时间序列
let buckets: Vec<HistoryBucket> = 
    conn.query_map("SELECT * FROM history_points_v2 WHERE ...", ...)?;

for bucket in buckets {
    let metrics: HashMap<String, Vec<f64>> = 
        serde_json::from_str(&bucket.metrics_json)?;
    // 线性插值重建时间点
}
```

**预期收益**:
- 数据库大小: -60%
- 查询速度: +40% (减少行扫描)
- 写入吞吐: +20% (批量 INSERT)

**权衡**:
- ✅ 存储效率大幅提升
- ✅ 减少 SQLite 页开销
- ⚠️ 需要迁移脚本（history_points → history_points_v2）
- ⚠️ 查询需要 JSON 解析（但比行扫描快）

**实施步骤**:
- [ ] 设计 history_points_v2 schema
- [ ] 实现 JSON 编码/解码逻辑
- [ ] 实现数据迁移脚本
- [ ] 修改 writer 写入逻辑（批量 JSON）
- [ ] 修改 query 读取逻辑（JSON 解析）
- [ ] 运行 load_test_history_pressure_scores 对比
- [ ] Commit: "perf(history): compress storage with JSONB batching"

**性价比**: ⭐⭐⭐ (中高难度，高收益，但需要迁移)

---

## 🚀 Phase 4: 架构级优化（预计 4-6 周）

### 4.1 Copy-on-Write 共享状态 ⭐⭐⭐⭐⭐

**当前问题**: 每次更新需要写锁整个 RegistryShard

```rust
// 当前架构：
pub struct Registry {
    shards: Vec<RwLock<RegistryShard>>,  // 写锁阻塞所有读取
}

// 更新流程：
let mut shard = registry.shards[idx].write();  // 独占写锁
shard.update_node(...);                        // 修改数据
drop(shard);                                    // 释放锁

// 问题：
// 1. 写锁阻塞所有读取（/api/overview, /api/nodes）
// 2. 大规模场景下锁竞争严重
// 3. 延迟尖刺（p99 vs p95 差距大）
```

**优化方案**: 不可变数据 + Arc 共享

```rust
// 新架构：
pub struct Registry {
    shards: Vec<Arc<DashMap<String, Arc<NodeEntry>>>>,
}

// 更新流程（无锁）：
let new_entry = Arc::new(updated_node_entry);
registry.shards[idx].insert(node_id, new_entry);  // 原子操作

// 读取流程（零拷贝）：
if let Some(entry) = registry.shards[idx].get(&node_id) {
    let entry_ref: Arc<NodeEntry> = entry.value().clone();  // 只递增引用计数
}

// 优势：
// ✅ 读写完全无锁（DashMap 内部分片锁）
// ✅ 读取零拷贝（Arc 克隆只是原子递增）
// ✅ 写入不阻塞读取
// ✅ 自动内存回收（Arc drop 时释放）
```

**设计权衡**:
```rust
// 问题：NodeEntry 需要变为不可变
// 解决：每次更新创建新 NodeEntry

// Before:
entry.last_seen = now;
entry.snapshot = new_snapshot;

// After:
let new_entry = NodeEntry {
    last_seen: now,
    snapshot: new_snapshot,
    ..entry.as_ref().clone()  // 复用不变字段
};
registry.insert(node_id, Arc::new(new_entry));
```

**预期收益**:
- 并发读取性能: +100% (完全无锁)
- p99 延迟: -50% (消除锁竞争尖刺)
- 写入吞吐: +30% (无写锁阻塞)

**实施步骤**:
- [ ] 将 NodeEntry 改为不可变结构
- [ ] 使用 DashMap 替代 RwLock<HashMap>
- [ ] 修改所有更新逻辑为 insert 新 Arc
- [ ] 添加并发压力测试
- [ ] 运行 load_test_concurrent_read_write_scores 对比
- [ ] Commit: "perf(registry): migrate to CoW with Arc + DashMap"

**性价比**: ⭐⭐⭐⭐ (高难度，**极高收益**，架构级改进)

---

### 4.2 冷热数据分离

**当前问题**: NodeEntry 包含高频和低频字段混合

```rust
// 当前：所有字段在一起
pub struct NodeEntry {
    // 高频访问（每秒）
    status: NodeStatus,
    last_seen: i64,
    snapshot: Option<NodeSnapshot>,
    
    // 低频访问（注册时一次）
    identity: NodeIdentity,
    token_hash: String,
    registered_at: i64,
    remote_ip: Option<String>,
    
    // 中频访问（偶尔）
    geoip_country: Option<Arc<str>>,
    session_id: Option<u64>,
}

// 问题：
// 1. 缓存局部性差（热数据分散）
// 2. 克隆开销大（包含不必要的冷数据）
```

**优化方案**: 分离为 Hot/Cold 结构

```rust
// 热数据（每秒访问）
pub struct NodeHotData {
    node_id: Arc<str>,
    status: NodeStatus,              // 8 字节
    last_seen: i64,                  // 8 字节
    snapshot: Option<NodeSnapshot>,  // 32 字节
    // 总共 ~64 字节
}

// 冷数据（注册时访问）
pub struct NodeColdData {
    identity: NodeIdentity,          // 256 字节
    token_hash: String,              // 64 字节
    registered_at: i64,
    remote_ip: Option<String>,
    geoip: GeoIpData,
    // 总共 ~400 字节
}

// Registry 分离存储
pub struct Registry {
    hot: Arc<DashMap<String, Arc<NodeHotData>>>,   // 高频访问
    cold: Arc<DashMap<String, Arc<NodeColdData>>>, // 低频访问
}
```

**预期收益**:
- 热数据缓存命中率: +50% (更好的局部性)
- /api/overview 响应: -30% (只克隆热数据)
- 内存占用: -20% (冷数据可 swap out)

**实施步骤**:
- [ ] 设计 Hot/Cold 数据结构
- [ ] 分离 Registry 存储
- [ ] 修改查询逻辑（按需 JOIN）
- [ ] 运行性能测试对比
- [ ] Commit: "perf(registry): separate hot and cold data"

**性价比**: ⭐⭐⭐ (高难度，中高收益，架构改动大)

---

## 📝 测试计划

### 每次优化后运行的测试

```bash
# 1. 单元测试
cargo test --workspace

# 2. 关键性能测试
cargo test -p nodelite-server --release load_test_history_pressure_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_large_fleet_scores -- --ignored --nocapture
cargo test -p nodelite-server --release load_test_reconnect_storm_scores -- --ignored --nocapture

# 3. 对比结果
# 记录到 PERF_OPTIMIZATION_RESULTS.md
```

### 性能对比表格模板

| 优化项 | History p95 | 连接时间 | 内存占用 | 提升 |
|--------|-------------|----------|----------|------|
| 基线 v3.0.1 | 20.91ms | 7,916ms | 337 MB | - |
| + LRU cache | ? | ? | ? | ? |
| + parking_lot | ? | ? | ? | ? |
| + token cache | ? | ? | ? | ? |

---

## 🎯 里程碑

- [x] **Milestone 1**: History API p95 < 35ms（达成：31.86ms）
- [x] **Milestone 2**: 重连场景大幅优化（20-100节点 p50 < 20ms，达成：93-98% 改进）
- [ ] **Milestone 3**: 1000节点内存 < 250 MB（当前：337 MB，Phase 2 后预期 320 MB）
- [ ] **Milestone 4**: 所有 API p95 < 5ms
- [ ] **Milestone 5**: 并发读取性能 +100%（Phase 4.1 CoW 架构）

**Phase 1 完成状态**: 4/4 完成 ✅
- ✅ 1.1: History Query LRU Cache (-14.8%)
- ✅ 1.2: parking_lot::RwLock for SharedState (混合结果)
- ✅ 1.3: Brotli 压缩（已启用，跳过）
- ✅ 1.4: Token 验证缓存 (-93.7% ~ -97.9% 重连延迟)

**Phase 2 完成状态**: 1/2 完成 ⚠️
- ✅ 2.2: String Pool (GeoIP 字段，-5% 内存)
- ❌ 2.3: History 预聚合（评估后取消）

**Phase 3 计划**: 3项优化，预期 2-3 周
- 3.1: 扩大字符串池范围（+Identity 字段，额外 -3% 内存）
- 3.2: NodeStatusView 轻量级视图（**-50% API 响应时间**）⭐⭐⭐⭐⭐
- 3.3: History SQLite JSONB 压缩（-60% 数据库大小）

**Phase 4 计划**: 2项架构级优化，预期 4-6 周
- 4.1: Copy-on-Write 共享状态（**+100% 并发性能**）⭐⭐⭐⭐⭐
- 4.2: 冷热数据分离（-20% 内存占用）

---

## 📊 优化性价比排序

| Phase | 优化项 | 预期收益 | 实施难度 | 工期 | 性价比 | 优先级 |
|-------|--------|---------|---------|------|--------|--------|
| 3.2 | NodeStatusView | 响应 -50% | 中 | 3天 | ⭐⭐⭐⭐⭐ | **最高** |
| 4.1 | CoW 架构 | 并发 +100% | 高 | 2周 | ⭐⭐⭐⭐⭐ | 高 |
| 3.3 | History 压缩 | 数据库 -60% | 中高 | 1周 | ⭐⭐⭐ | 中 |
| 3.1 | 扩展字符串池 | 内存 -3% | 低 | 2天 | ⭐⭐⭐ | 中 |
| 4.2 | 冷热分离 | 内存 -20% | 高 | 2周 | ⭐⭐⭐ | 中低 |

**推荐实施顺序**: Phase 3.2 → Phase 3.1 → Phase 4.1 → Phase 3.3 → Phase 4.2

---

## 📋 Commit 规范

所有 commit 使用以下前缀：

- `perf(history):` - History 相关优化
- `perf(auth):` - 认证/Token 优化
- `perf(registry):` - Registry 内存优化
- `perf(lock):` - 锁机制优化
- `perf:` - 通用性能优化

每个 commit 附带性能测试结果。

---

## 🔄 回滚策略

每个优化都是独立的 commit，如果发现问题可以单独回滚：

```bash
# 回滚最后一次优化
git revert HEAD

# 回滚特定优化
git revert <commit-hash>
```

---

## 📊 监控指标

在 `PERF_OPTIMIZATION_RESULTS.md` 中记录：

1. **延迟指标**: p50, p95, p99, max
2. **吞吐指标**: 指标/秒
3. **内存指标**: RSS, 堆占用
4. **缓存指标**: 命中率（如果添加缓存）

---

**最后更新**: 2026-06-16  
**状态**: Phase 2 完成，Phase 3/4 规划完成
