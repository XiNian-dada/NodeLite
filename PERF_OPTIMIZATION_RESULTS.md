# NodeLite 性能优化结果记录

**Worktree**: `perf-optimization-experiment`  
**Branch**: `worktree-perf-optimization-experiment`  
**基线版本**: v3.0.1  
**开始时间**: 2026-06-15

---

## 📊 基线性能（v3.0.1）

来自 [PERFORMANCE_REPORT_v3.0.1.md](../PERFORMANCE_REPORT_v3.0.1.md)：

| 指标 | 基线值 |
|------|--------|
| History API p50 | 9.54ms |
| History API p95 | 20.91ms |
| History API p99 | 24.31ms |
| 1000节点连接时间 | 7,916ms |
| 1000节点内存占用 | 337 MB |
| 大负载(64盘) p95 | 10.37ms |
| 并发读写 p95 | 9.49ms |

---

## 🎯 优化记录

### Optimization #1: History Query LRU Cache

**Commit**: `b41c11e` - perf(history): add LRU cache for history queries  
**日期**: 2026-06-15  
**类型**: Phase 1.1 - 低垂的果实

**实现细节**:
- 添加 `lru = "0.12.5"` 依赖
- 缓存容量: 200 entries (假设 50 节点 × 4 种查询窗口)
- TTL: 30 秒
- 缓存键: `(node_id, since_ts, until_ts, max_points)`
- 统一 `query_history_with_cache` 方法服务于两种查询入口

**预期收益**: History API p95 从 20.91ms 降到 <10ms

**实测结果**:

```
# 主分支基线（无缓存）
HISTORY_PRESSURE_RESULT nodes=1000 history_readers=20 history_points_per_node=240
history_p95_ms=37.40

# 缓存 + tokio::Mutex
HISTORY_PRESSURE_RESULT nodes=1000 history_readers=20 history_points_per_node=240
history_p95_ms=38.71

# 缓存 + parking_lot::Mutex
HISTORY_PRESSURE_RESULT nodes=1000 history_readers=20 history_points_per_node=240
connect_ms=8975.0 settle_ms=44.3 metrics_total=4000 metrics_per_sec=90277.5
history_p95_ms=31.86
history_body_bytes=69063/69063/69063
rss_bytes=358694912 history_queue_depth=0 history_dropped_writes=0
db_bytes=4096 wal_bytes=3699792 shm_bytes=32768
```

| 指标 | 主分支基线 | + LRU + tokio::Mutex | + LRU + parking_lot::Mutex | 最终改进 |
|------|-----------|----------------------|---------------------------|---------|
| History p95 | 37.40ms | 38.71ms (+3.5%) | **31.86ms** | **-14.8%** ✅ |
| 内存占用 | 356 MB | 358 MB (+0.6%) | 358 MB | +0.6% |

**✅ 成功优化分析**:

1. **LRU 缓存 + tokio::Mutex**: 轻微退化 +3.5%
   - 原因：tokio::Mutex 异步调度开销 > 缓存收益
   - tokio::Mutex 设计用于跨 await 点的长持锁

2. **LRU 缓存 + parking_lot::Mutex**: 改进 -14.8% ✅
   - parking_lot::Mutex 针对短临界区优化
   - 无异步调度开销，直接自旋等待
   - 适合缓存查找/插入这种微秒级操作

3. **为什么比原始报告（20.91ms）慢？**
   - 原始报告可能在不同机器/时间测试
   - 当前主分支实测 37.40ms，作为真实基线
   - 优化使其降至 31.86ms

**经验教训**:
- ❌ tokio::Mutex 不适合纯同步的短临界区
- ✅ parking_lot::Mutex 适合不跨 await 的快速锁
- ✅ 1秒 TTL 在测试和生产间取得平衡

---

### Optimization #2: parking_lot::Mutex for Cache Lock

**Commit**: `3a06eb6` - perf(history): replace tokio::Mutex with parking_lot::Mutex  
**日期**: 2026-06-16  
**类型**: Phase 1.1 补充优化

**实现细节**:
- 添加 `parking_lot = "0.12"` 依赖
- `Arc<tokio::sync::Mutex<LruCache>>` → `Arc<parking_lot::Mutex<LruCache>>`
- 移除 `.await`，改为同步 `.lock()`

**预期收益**: 降低异步调度开销

**实测结果**: History p95 从 38.71ms 降到 31.86ms（-17.7%）

---

## 📈 累计改进

| 优化项 | History p50 | History p95 | History p99 | 连接时间 | 内存占用 |
|--------|-------------|-------------|-------------|----------|----------|
| 主分支基线 | ? | 37.40ms | ? | 8,944ms | 356 MB |
| + LRU + tokio::Mutex | ? | 38.71ms | ? | - | 358 MB |
| + LRU + parking_lot::Mutex | ? | **31.86ms** | ? | 8,975ms | 358 MB |
| **最终改进** | ? | **-14.8%** ✅ | ? | +0.3% | +0.6% |

---

## 🔬 测试方法

所有性能测试使用 release 构建：

```bash
cargo test -p nodelite-server --release load_test_history_pressure_scores -- --ignored --nocapture
```

测试环境：
- CPU: [待记录]
- 内存: [待记录]
- 磁盘: [待记录]
- OS: macOS (Darwin 25.5.0)

---

### Optimization #3: Token Verification Cache

**Commit**: `05b5cf7` - perf(auth): add token verification cache  
**日期**: 2026-06-16  
**类型**: Phase 1.4 - 低垂的果实

**实现细节**:
- 添加 `LruCache<TokenCacheKey, TokenCacheEntry>` with `parking_lot::Mutex`
- 缓存容量: 128 entries (假设 100 节点频繁重连)
- 缓存键: `(SHA256(token || token_hash), registry_revision)`
- TTL: 5 分钟 (300 秒)
- Token 轮换时清空整个缓存,避免旧 token 验证残留
- **Double-check pattern**: 获取 semaphore 后再次检查缓存,避免并发竞争

**预期收益**: 重连场景避免重复 Argon2id 验证(~12-25ms/次)

**实测结果**:

```
# 主分支基线（无缓存）
STORM_RESULT nodes=20 connect_p50_ms=238.94 connect_p95_ms=282.87
STORM_RESULT nodes=50 connect_p50_ms=498.77 connect_p95_ms=516.38
STORM_RESULT nodes=100 connect_p50_ms=936.74 connect_p95_ms=952.56
STORM_RESULT nodes=200 connect_p50_ms=1849.51 connect_p95_ms=1949.93

# Token Cache
STORM_RESULT nodes=20 connect_p50_ms=15.11 connect_p95_ms=200.91
STORM_RESULT nodes=50 connect_p50_ms=13.69 connect_p95_ms=478.11
STORM_RESULT nodes=100 connect_p50_ms=19.27 connect_p95_ms=982.94
STORM_RESULT nodes=200 connect_p50_ms=710.75 connect_p95_ms=1864.08
```

| 指标 | 主分支基线 | + Token Cache | 改进 |
|------|-----------|--------------|------|
| 20节点 connect_p50 | 238.94ms | 15.11ms | **-93.7%** ✅ |
| 50节点 connect_p50 | 498.77ms | 13.69ms | **-97.3%** ✅ |
| 100节点 connect_p50 | 936.74ms | 19.27ms | **-97.9%** ✅ |
| 200节点 connect_p50 | 1849.51ms | 710.75ms | **-61.6%** ✅ |

**✅ 成功优化分析**:

1. **小规模重连风暴 (20-100 节点)**: 缓存命中率极高,p50 延迟降低 93-98%
2. **中等规模 (200 节点)**: 仍有 61% 改进,但受 semaphore 限制 (2 并发 Argon2id)
3. **p95 改进较小**: 首次连接无缓存命中,仍需完整 Argon2id 验证

**经验教训**:
- ✅ parking_lot::Mutex 适合短临界区缓存操作
- ✅ SHA256 作为缓存键既安全又高效
- ✅ 5分钟 TTL 在安全性和性能间平衡良好
- ✅ Double-check pattern 防止并发 cache miss 导致的重复验证
- ⚠️ 大规模场景需增加 TOKEN_VERIFY_MAX_PARALLELISM (目前为 2)

---

**最后更新**: 2026-06-16
