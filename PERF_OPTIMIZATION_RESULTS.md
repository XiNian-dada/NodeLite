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
HISTORY_PRESSURE_RESULT nodes=1000 history_readers=20 history_points_per_node=240
connect_ms=9292.1 settle_ms=54.3 metrics_total=4000 metrics_per_sec=73692.5
history_p95_ms=38.71
history_body_bytes=69063/69063/69063
rss_bytes=358891520 history_queue_depth=0 history_dropped_writes=0
db_bytes=4096 wal_bytes=3708032 shm_bytes=32768
```

| 指标 | 基线 v3.0.1 | + LRU cache | 变化 |
|------|-------------|-------------|------|
| History p50 | 9.54ms | ? | ? |
| History p95 | 20.91ms | **38.71ms** | **+85% (⚠️ 退化)** |
| History p99 | 24.31ms | ? | ? |

**⚠️ 性能退化分析**:

LRU 缓存反而导致 p95 从 20.91ms 升至 38.71ms（+85%）。可能原因：

1. **Mutex 竞争**: 每次查询都要获取 `Arc<Mutex<LruCache>>` 锁，20 个并发读者争抢同一把锁
2. **缓存未命中开销**: 测试场景中每个节点只查询 4 次，缓存命中率低，额外的锁开销没有被缓存收益抵消
3. **1秒 TTL 过短**: 查询间隔可能 > 1s，导致缓存总是过期
4. **测试场景不代表生产**: 测试是 20 个独立节点各查 4 次，生产场景是多用户反复查同一节点

**后续行动**:
- [ ] 运行 API surface 测试对比（包含重复查询场景）
- [ ] 考虑使用 `parking_lot::Mutex` 替代 `tokio::sync::Mutex`
- [ ] 或者移除缓存，转而优化 SQLite 查询本身

---

## 📈 累计改进

| 优化项 | History p50 | History p95 | History p99 | 连接时间 | 内存占用 |
|--------|-------------|-------------|-------------|----------|----------|
| 基线 v3.0.1 | 9.54ms | 20.91ms | 24.31ms | 7,916ms | 337 MB |
| + LRU cache | ? | **38.71ms** | ? | - | 358 MB |
| **退化** | ? | **+85%** ⚠️ | ? | - | +6% |

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

**最后更新**: 2026-06-15
