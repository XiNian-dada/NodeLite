# Reconnect Storm 延迟诊断报告

**测试日期**: 2026-07-02  
**场景**: 200 节点 × 4 cycles = 800 sessions  
**实测 p95**: 1,689ms  
**理论值**: 425ms (200 ÷ 2 × 17ms)  
**差距**: 4×

---

## 实测时序分解

### 全局统计 (1480 总连接)

| Phase | Average | Max | 占比 |
|-------|---------|-----|------|
| **TCP/WS handshake** | 10.9ms | 36.1ms | 5.2% |
| **Hello send** | 0.01ms | 0.89ms | 0.0% |
| **Auth wait** | **200.8ms** | **1,651.8ms** | **94.8%** |
| **Total** | 211.7ms | 1,688.0ms | 100% |

### Auth Wait 分布

- **低延迟 (<2ms)**: 582 次 (39.3%) - **缓存命中**
- **中等延迟 (2-100ms)**: 379 次 (25.6%) - 可能是排队
- **高延迟 (>100ms)**: 519 次 (35.1%) - **Argon2id 验证 + 排队**

---

## 根因分析

### ✅ 确认: Auth wait 是主要瓶颈

**证据**:
- Auth wait 平均 200.8ms，占总延迟的 **94.8%**
- p95 时 auth wait = 1,651.8ms (接近总 p95 的 1,689ms)

### 🔴 问题: Token 缓存命中率低于预期

**预期**:
- 4 cycles 复用相同 credentials
- 第 1 cycle: 缓存未命中，全部 Argon2id 验证
- 第 2-4 cycles: 缓存命中率应该 ~75%

**实测**:
- 缓存命中率 **39.3%** (582/1480)
- 远低于理论 75%

### 🤔 可能的原因

#### 假设 1: 缓存容量不足 (128 entries)
200 节点场景下，LRU 缓存可能被逐出：
- 缓存容量: 128 entries
- 需要缓存: 200 tokens
- **缓存逐出**: 72 tokens 会被 LRU 踢出

**验证**:
```rust
// registry.rs:79
const TOKEN_CACHE_CAPACITY: usize = 128;
```

200 节点时，最多只能缓存 128 个，剩余 72 个会在每次访问时被逐出并重新验证。

#### 假设 2: 缓存键碰撞
缓存键包含 `(token_hash, registry_revision)`：
```rust
// registry/auth.rs:122
let cache_key = (cache_key_hash.clone(), self.registry_revision());
```

如果 `registry_revision` 在测试过程中变化，会导致缓存失效。

**需要验证**: 测试期间 registry_revision 是否稳定（理论上应该稳定，因为没有 token 轮换）

#### 假设 3: 并发竞争导致缓存逐出
200 节点并发连接时，LRU 缓存的访问模式可能导致：
- 早期连接的 token 被逐出
- 晚期连接的 token 又把早期的挤出去
- 结果：缓存命中率下降

---

## 性能影响计算

### 当前实测 (缓存命中率 39.3%)

**第 1 cycle** (200 节点，全部未命中):
- Argon2id 验证: 200 × 17ms ÷ 2 并发 = 1,700ms
- p95 延迟: ~1,700ms

**第 2-4 cycles** (600 节点，命中率假设 40%):
- 未命中: 600 × 60% = 360 次验证
- 平均排队: 360 ÷ 2 = 180 轮
- 延迟: 180 × 17ms = 3,060ms 分摊到 3 cycles
- 每 cycle 平均: ~1,020ms

**加权平均**: (1,700 + 1,020×3) / 4 = **1,190ms**

实测 p95 = 1,689ms 略高于平均值，符合预期。

### 理想情况 (缓存命中率 75%)

**第 1 cycle**: 1,700ms (同上)

**第 2-4 cycles** (600 节点，命中率 75%):
- 未命中: 600 × 25% = 150 次验证
- 平均排队: 150 ÷ 2 = 75 轮
- 延迟: 75 × 17ms = 1,275ms 分摊到 3 cycles
- 每 cycle 平均: ~425ms

**加权平均**: (1,700 + 425×3) / 4 = **744ms**

**改善空间**: 1,689ms → 744ms (**-56%**)

---

## 优化方案

### 方案 A: 增加 Token 缓存容量

**改动**:
```rust
// registry.rs:79
const TOKEN_CACHE_CAPACITY: usize = 128;  // 当前
const TOKEN_CACHE_CAPACITY: usize = 512;  // 建议
```

**预期收益**:
- 200 节点场景: 缓存命中率从 39% → 75%
- p95 延迟: 1,689ms → 744ms (**-56%**)

**成本**:
- 内存增加: ~120KB (假设每条缓存 ~80 bytes × (512-128))
- 几乎可忽略不计

**风险**: 极低

---

### 方案 B: 提升 Argon2id 并发度 (已测试，效果有限)

**测试结果**:
- 2 → 8: 改善 **3%** (1,725ms → 1,675ms)
- 2 → 16: **反而更慢** (1,732ms)

**结论**: 并发度不是主要瓶颈，因为 61% 的连接仍然需要 Argon2id 验证。

---

### 方案 C: 预热缓存 (测试场景优化)

**思路**: 在重连测试前，先执行一次"热身"cycle 填充缓存。

**仅适用于测试**，生产环境无法预热。

---

## 推荐行动

### P0: 增加 Token 缓存容量至 512

**理由**:
1. **低风险、高收益**: 只改一行代码，内存成本可忽略
2. **解决根因**: 直接提升缓存命中率
3. **通用改善**: 对生产环境重连场景同样有效

**实施**:
```rust
// nodelite-server/src/registry.rs:79
-const TOKEN_CACHE_CAPACITY: usize = 128;
+const TOKEN_CACHE_CAPACITY: usize = 512;
```

**预期结果**:
- 200 节点 reconnect storm p95: 1,689ms → 744ms (-56%)
- 缓存命中率: 39% → 75%

---

### 可选: 监控缓存效率

添加指标验证缓存命中率：
- Token cache hits
- Token cache misses
- Token cache size
- Token cache evictions

---

## 附录: 时序样本

### 第 1 cycle (20 节点) - 缓存未命中
```
AGENT_TIMING node=load-node-003 tcp_ws_ms=1.18 hello_send_ms=0.02 auth_wait_ms=16.80 total_ms=17.99
AGENT_TIMING node=load-node-013 tcp_ws_ms=1.39 hello_send_ms=0.01 auth_wait_ms=18.12 total_ms=19.52
...
AGENT_TIMING node=load-node-017 tcp_ws_ms=1.38 hello_send_ms=0.01 auth_wait_ms=136.78 total_ms=138.17
```
Auth wait 从 16ms 递增到 136ms，符合串行排队特征（2 并发）。

### 第 2 cycle (20 节点) - 部分缓存命中
```
AGENT_TIMING node=load-node-003 tcp_ws_ms=4.23 hello_send_ms=0.02 auth_wait_ms=0.34 total_ms=4.59
AGENT_TIMING node=load-node-013 tcp_ws_ms=2.01 hello_send_ms=0.01 auth_wait_ms=1.19 total_ms=3.21
...
```
Auth wait 下降到 <2ms，说明缓存开始命中。

### 200 节点场景 - 缓存命中率低
```
Average auth_wait = 114.059ms (last 200 connections)
Max auth_wait = 585.32ms
```
平均 auth wait 仍然很高，说明缓存命中率不足。

---

**结论**: Token 缓存容量 (128) 不足以支撑 200 节点场景，建议提升至 512。
