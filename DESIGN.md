# 浏览器 WebSocket 集中 diff 广播

本文描述当前实现。早期每个浏览器独立重算 diff 的方案已由集中后台任务替代；
早期 200 节点、约 4 个浏览器的观测属于历史基准，不能作为当前性能结果。

## 数据流

`SharedState` 的节点 revision 与脏信号触发 `state/browser_broadcast.rs` 中的
`spawn_browser_incremental_task`。后台任务按 UI 刷新间隔合并变更，每批只生成一次
节点列表、diff 和概览；序列化的帧由 `Arc` 共享并广播给浏览器会话。

```text
节点变更 → revision / 脏信号 → 集中 diff 任务 → 共享序列化批次
                                                    ↓
                                        ws/browser.rs 的各浏览器会话
```

`ws/browser.rs` 负责转发批次、心跳与连接生命周期，不再逐连接计算节点 diff。
增量通道容量为 4 个批次，已被所有订阅者读取的批次可以释放；慢会话最多保留
有限批次，落后时通过 `InitialState` 重同步。

## 快照与 revision

连接建立和 `Lagged` 重同步复用单飞构建的全量帧。缓存只有在 revision 不早于
最近已发布的增量且仍在新鲜度窗口内时才可复用。全量帧携带其节点 revision
baseline；会话丢弃 `revision <= baseline` 的排队增量，避免旧批次覆盖较新快照。

无订阅者时后台任务跳过 diff 并释放基线与全量帧。任务使用弱引用，最后一个
`SharedState` 释放后退出。发送大型批次时会话仍处理 Ping、Close 和发送超时。

## 验证与取舍

Rust 单元及浏览器 WebSocket 集成测试覆盖全量、增量、离线、重同步、revision、
心跳和未认证拒绝。性能与内存的测量方法见 [历史查询矩阵](docs/history-query-matrix.md)
和 [基准记录](docs/benchmarks.md)；测量结果仅适用于各记录标明的环境。

集中计算将重复 diff 从每浏览器一次减少为每批一次；共享帧减少复制，但慢连接
仍占用有界的批次内存。广播落后时重新发送全量会增加瞬时流量，这是恢复一致性的取舍。
