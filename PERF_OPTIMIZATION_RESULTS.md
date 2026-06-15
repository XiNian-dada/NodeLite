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

*等待测试完成...*

---

## 📈 累计改进

| 优化项 | History p50 | History p95 | History p99 | 连接时间 | 内存占用 |
|--------|-------------|-------------|-------------|----------|----------|
| 基线 v3.0.1 | 9.54ms | 20.91ms | 24.31ms | 7,916ms | 337 MB |
| + LRU cache | ? | ? | ? | - | - |

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
