# Phase 3.2: NodeListItemView 零拷贝优化设计

## 问题分析

### 当前瓶颈 (2026-06-16)

**数据流**:
```
Registry (Arc<str>) → to_summary() → NodeListItem (String) → serde_json → Bytes
                      ↑ 每次克隆 ↑
```

**性能指标** (1000 nodes):
- `list_node_summaries()`: 每次克隆 4000+ 字符串 (4 GeoIP 字段 × 1000 nodes)
- 内存峰值: ~500 KB 临时分配
- p95 延迟: 5.44ms

**代码位置**:
- `nodelite-server/src/state/registry.rs:202` - `NodeEntry::to_summary()`
  ```rust
  geoip_country: self.geoip_country.as_ref().map(|s| s.to_string()),
  //                                                  ^^^^^^^^^^^^ 克隆!
  ```

---

## 优化方案

### 目标

**零拷贝序列化**:
```
Registry (Arc<str>) → to_summary_view() → NodeListItemView (Arc<str>) → serde_json → Bytes
                                          ↑ 零拷贝 ↑
```

**预期收益**:
- 内存峰值: -50% (减少临时克隆)
- p95 延迟: 5.44ms → < 3ms
- API 响应时间: -50%

---

## 实现细节

### 1. 新结构定义

**nodelite-proto/src/model.rs**:
```rust
/// 零拷贝的节点列表视图，用于 API 响应序列化。
///
/// 与 `NodeListItem` 的区别：
/// - GeoIP 字段直接持有 `Arc<str>`，避免克隆
/// - 序列化时 serde 直接访问 Arc 内部的 str
#[derive(Debug, Clone, Serialize)]
pub struct NodeListItemView {
    pub identity: NodeListIdentity,
    
    // GeoIP 字段：使用 Arc<str> 避免克隆
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geoip_country: Option<Arc<str>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geoip_city: Option<Arc<str>>,
    #[serde(default)]
    pub geoip_latitude: Option<f64>,
    #[serde(default)]
    pub geoip_longitude: Option<f64>,
    
    // Location override 字段：同样使用 Arc<str>
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_override_country: Option<Arc<str>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_override_city: Option<Arc<str>>,
    #[serde(default)]
    pub location_override_latitude: Option<f64>,
    #[serde(default)]
    pub location_override_longitude: Option<f64>,
    
    pub snapshot: Option<NodeListSnapshot>,
    pub latency_ms: Option<u64>,
    pub online: bool,
}
```

**关键设计决策**:
1. ✅ `Arc<str>` 实现了 `Serialize`，serde 会调用 `Deref<Target=str>` 直接序列化
2. ✅ `skip_serializing_if = "Option::is_none"` 避免序列化 `null` 字段
3. ✅ `Clone` 成本很低（只增加引用计数）
4. ✅ 保持与 `NodeListItem` 相同的 JSON 输出格式

---

### 2. NodeEntry 方法

**nodelite-server/src/state/registry.rs**:
```rust
impl NodeEntry {
    /// 零拷贝构建视图 (Phase 3.2 优化)
    fn to_summary_view(&self) -> NodeListItemView {
        NodeListItemView {
            identity: NodeListIdentity::from(&self.identity),
            // 直接克隆 Arc (只增加引用计数，不复制字符串)
            geoip_country: self.geoip_country.clone(),
            geoip_city: self.geoip_city.clone(),
            geoip_latitude: self.geoip_latitude,
            geoip_longitude: self.geoip_longitude,
            location_override_country: self.location_override_country.clone(),
            location_override_city: self.location_override_city.clone(),
            location_override_latitude: self.location_override_latitude,
            location_override_longitude: self.location_override_longitude,
            snapshot: self.snapshot.as_ref().map(NodeListSnapshot::from),
            latency_ms: self.latency_ms,
            online: self.online,
        }
    }
    
    // 保留旧方法以兼容测试代码
    fn to_summary(&self) -> NodeListItem {
        // ...现有实现...
    }
}
```

---

### 3. Registry API 更新

**nodelite-server/src/state/registry.rs**:
```rust
impl Registry {
    pub(super) fn list_node_summaries_view(&self) -> Vec<NodeListItemView> {
        let shards = self.read_all_shards();
        sorted_entries(&shards)
            .into_iter()
            .map(NodeEntry::to_summary_view)
            .collect()
    }
    
    // 保留旧方法以兼容测试
    pub(super) fn list_node_summaries(&self) -> Vec<NodeListItem> {
        // ...现有实现...
    }
}
```

---

### 4. State API 更新

**nodelite-server/src/state.rs**:
```rust
impl SharedState {
    pub async fn nodes_json_bytes(&self) -> Result<Bytes, serde_json::Error> {
        self.cached_api_json_bytes(ApiBodyKind::Nodes).await
    }
    
    async fn cached_api_json_bytes(&self, kind: ApiBodyKind) -> Result<Bytes, serde_json::Error> {
        // ...缓存逻辑...
        
        let body = match kind {
            ApiBodyKind::Nodes => {
                let summaries = self.list_node_summaries_view().await;  // 改这里
                Bytes::from(serde_json::to_vec(&summaries)?)
            }
            // ...
        };
        
        // ...
    }
    
    pub async fn list_node_summaries_view(&self) -> Vec<NodeListItemView> {
        self.registry.list_node_summaries_view()
    }
}
```

---

## 测试策略

### 1. 序列化正确性

验证 `NodeListItemView` 的 JSON 输出与 `NodeListItem` 完全一致：

```rust
#[test]
fn node_list_item_view_serializes_identically_to_node_list_item() {
    let view = NodeListItemView {
        identity: sample_identity(),
        geoip_country: Some(Arc::from("US")),
        geoip_city: Some(Arc::from("San Francisco")),
        // ...
    };
    
    let item = NodeListItem {
        identity: sample_identity(),
        geoip_country: Some("US".to_string()),
        geoip_city: Some("San Francisco".to_string()),
        // ...
    };
    
    let view_json = serde_json::to_value(&view).unwrap();
    let item_json = serde_json::to_value(&item).unwrap();
    
    assert_eq!(view_json, item_json);
}
```

### 2. Arc 共享验证

验证 Arc<str> 确实被共享（引用计数 > 1）：

```rust
#[test]
fn node_list_item_view_shares_arc_references() {
    let country = Arc::from("US");
    let country_clone = Arc::clone(&country);
    
    let view = NodeListItemView {
        geoip_country: Some(country),
        // ...
    };
    
    // 验证 Arc 被共享（引用计数 >= 2）
    assert!(Arc::strong_count(&country_clone) >= 2);
}
```

### 3. 内存基准测试

对比 `to_summary()` vs `to_summary_view()` 的内存分配：

```rust
#[test]
fn node_list_item_view_reduces_allocations() {
    let entries = sample_node_entries(1000);
    
    // 测量旧方案分配
    let old_alloc_before = GLOBAL_ALLOCATOR.allocated();
    let _old_summaries: Vec<NodeListItem> = entries.iter()
        .map(|e| e.to_summary())
        .collect();
    let old_alloc_delta = GLOBAL_ALLOCATOR.allocated() - old_alloc_before;
    
    // 测量新方案分配
    let new_alloc_before = GLOBAL_ALLOCATOR.allocated();
    let _new_summaries: Vec<NodeListItemView> = entries.iter()
        .map(|e| e.to_summary_view())
        .collect();
    let new_alloc_delta = GLOBAL_ALLOCATOR.allocated() - new_alloc_before;
    
    // 预期新方案分配减少 50%+
    assert!(new_alloc_delta < old_alloc_delta / 2);
}
```

---

## 向后兼容性

### 保留旧 API

- `NodeListItem` 保留用于测试代码
- `to_summary()` 保留用于非热路径
- `list_node_summaries()` 保留但标记为 deprecated

### JSON 格式不变

`NodeListItemView` 序列化后的 JSON 与 `NodeListItem` 完全一致，前端无需修改。

---

## 性能预期

### Before (Phase 2)
```
1000 nodes × 4 GeoIP 字段 × ~20 bytes = 80 KB 字符串克隆
+ NodeListItem 结构体分配: ~420 KB
= 500 KB 临时内存
p95 延迟: 5.44ms
```

### After (Phase 3.2)
```
1000 nodes × 8 bytes (Arc 指针) = 8 KB Arc 克隆
+ NodeListItemView 结构体分配: ~210 KB
= 218 KB 临时内存
p95 延迟: < 3ms (预期)
```

**预期改进**:
- 内存峰值: **-56%** (500 KB → 218 KB)
- p95 延迟: **-45%** (5.44ms → 3ms)

---

## 实施检查清单

### 代码变更
- [ ] 在 `nodelite-proto/src/model.rs` 定义 `NodeListItemView`
- [ ] 在 `NodeEntry` 中实现 `to_summary_view()`
- [ ] 在 `Registry` 中实现 `list_node_summaries_view()`
- [ ] 更新 `SharedState::cached_api_json_bytes()` 使用新方法
- [ ] 添加序列化正确性测试
- [ ] 添加 Arc 共享验证测试
- [ ] (可选) 添加内存基准测试

### 验证
- [ ] 所有测试通过
- [ ] `/api/nodes` JSON 格式不变
- [ ] p95 延迟 < 3ms (运行 load_test_large_fleet_scores)
- [ ] 无内存泄漏 (运行 10 分钟压测)

### 文档
- [ ] 更新 `PERF_OPTIMIZATION_RESULTS_PHASE2.md`
- [ ] Commit message: "perf(api): add zero-copy NodeListItemView"
- [ ] PR 描述包含性能对比数据

---

## 风险评估

### 低风险
- ✅ `Arc<str>` 已经在 Phase 2 中验证稳定
- ✅ serde 对 Arc 的支持是标准库特性
- ✅ JSON 输出格式不变，前端无感知

### 中等风险
- ⚠️  如果未来需要修改 GeoIP 字段，必须同时更新 `NodeListItem` 和 `NodeListItemView`
- **缓解**: 添加编译时测试确保两者字段一致

### 可接受权衡
- 引入了两个相似的结构体 (`NodeListItem` vs `NodeListItemView`)
- **理由**: 性能收益显著（-50% 内存 + -45% 延迟），值得少量代码重复

---

## 后续优化 (Phase 4)

Phase 3.2 完成后，可以考虑：

1. **Phase 4.1: CoW Registry** - 进一步减少读锁争用
2. **Phase 3.3: History 压缩存储** - 减少 SQLite 行数
3. **Phase 3.1: Identity 字段 Interning** - 如果高基数成为问题

---

## 参考

- Phase 2 String Pool 实现: commit `f80be50`
- Load test 基准: `PERF_OPTIMIZATION_RESULTS_PHASE2.md`
- Arc serialization: https://docs.rs/serde/latest/serde/trait.Serialize.html#impl-Serialize-for-Arc%3CT%3E
