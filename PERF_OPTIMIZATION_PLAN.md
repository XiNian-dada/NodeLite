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
- [ ] 添加查询分析日志
- [ ] 实现 LRU 缓存层
- [ ] 运行性能测试对比
- [ ] Commit: "perf(history): add LRU cache for history queries"

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

**当前状态**: 可能只启用了 gzip

**优化方向**:
```rust
// tower-http 已经在依赖中，检查是否启用 Brotli
// Brotli 对 JSON 的压缩率比 gzip 高 15-20%

// 250KB JSON → ~75KB (gzip) → ~60KB (brotli)
```

**预期收益**: 大负载响应时间降低 20-30%

**实施步骤**:
- [ ] 检查 `tower-http` compression 配置
- [ ] 确保 Brotli 已启用
- [ ] 测试不同压缩级别的性能
- [ ] Commit: "perf: ensure brotli compression is enabled"

---

## 🚀 Phase 2: 结构性优化（预计 3-4 周）

### 2.1 Token 验证缓存

**当前状态**: 每次连接都运行 Argon2id 验证（CPU 密集）

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

**预期收益**: 1000节点连接时间降到 < 5s

**实施步骤**:
- [ ] 设计 TokenCache 结构
- [ ] 实现 LRU 缓存逻辑
- [ ] 集成到 registry::authorize_identity
- [ ] 添加监控指标（缓存命中率）
- [ ] 运行重连风暴测试对比
- [ ] Commit: "perf(auth): add token verification cache"

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

## 🔬 Phase 3: 高级优化（长期）

### 3.1 时序数据库评估

**考虑场景**: 如果历史数据规模继续增长（10K+ 节点，数月保留）

**候选方案**:
1. **TimescaleDB**（PostgreSQL 扩展）
   - 优势：SQL 兼容，自动分区，压缩
   - 缺点：需要 PostgreSQL

2. **ClickHouse**
   - 优势：极快的聚合查询
   - 缺点：学习曲线，运维复杂

3. **保持 SQLite + 优化**
   - 优势：零依赖，简单
   - 缺点：性能上限

**决策点**: 当 History API p95 > 50ms 或数据库 > 10GB 时重新评估

---

### 3.2 前端虚拟滚动

**场景**: 1000+ 节点时 DOM 渲染性能

**优化方向**:
```typescript
// 使用 vue-virtual-scroller
// 只渲染可见区域的节点（~20-30 个）
// 预期：1000 节点渲染时间从 5s → < 1s
```

**前提**: 需要先运行前端 DOM 性能测试确认是否需要

---

### 3.3 WebSocket 消息批处理

**场景**: 极高频率更新（每节点 > 1Hz）

**优化方向**:
```rust
// 批量发送 WebSocket 消息
// 100ms 窗口内的多个 NodeUpsert 合并为一条消息
// 减少网络往返和前端解析开销
```

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

- [ ] **Milestone 1**: History API p95 < 10ms
- [ ] **Milestone 2**: 1000节点连接时间 < 5s
- [ ] **Milestone 3**: 1000节点内存 < 250 MB
- [ ] **Milestone 4**: 所有 API p95 < 5ms

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

**最后更新**: 2026-06-15  
**状态**: Phase 1 进行中
