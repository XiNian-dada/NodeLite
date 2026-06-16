# 性能优化 #1: 浏览器 WebSocket 集中 diff 广播

## 当前瓶颈

`ws/browser.rs` 当前为每个浏览器连接独立订阅脏信号、重算节点列表、执行 diff。
在 N 节点、M 浏览器连接场景下，每秒产生：
- **M × list_node_summaries** 调用 → M 次 registry RwLock 读锁竞争 + M×N 次克隆
- **M × diff_node_lists** 计算 → M×O(N) 遍历 + HashMap 构建
- **M × overview_snapshot** 调用 → M 次聚合统计重算

报告实测(200 节点、~4 浏览器)：4 连接共发出 ~40 upserts/s，单次节点变化被 diff 4 次。

## 优化方案

**集中广播架构**：用单个后台任务订阅脏信号，每秒最多一次执行 diff，把增量消息
广播到专用 channel；各浏览器会话订阅该 channel，直接转发收到的消息(零锁、零 diff)。

```
[SharedState 脏信号]
       ↓
[集中 diff 任务] ← 1× list_node_summaries (去抖 1s)
       ↓ diff_node_lists → Vec<BrowserIncrementalUpdate>
[broadcast channel: 带 revision 的增量消息]
    /     |      \
 [会话1] [会话2] [会话3] ← 直接转发,无锁无 diff
```

收益：
- 锁竞争 O(M) → O(1)
- diff 计算 O(M×N) → O(N)
- 概览重算 O(M) → O(1)

## 实现步骤

### 1. 在 `state.rs` 中新增增量广播 channel

```rust
pub(crate) struct BrowserIncrementalUpdate {
    revision: u64,
    message: Arc<BrowserMessage>,
}

pub(crate) struct SharedState {
    // ... existing fields
    /// 集中 diff 任务广播的增量消息,各浏览器会话订阅后直接转发(零锁、零 diff)。
    browser_incremental_tx: broadcast::Sender<BrowserIncrementalUpdate>,
}

impl SharedState {
    pub(crate) fn subscribe_browser_incremental(&self) -> broadcast::Receiver<BrowserIncrementalUpdate> {
        self.browser_incremental_tx.subscribe()
    }
}
```

容量设为 256(每节点变化 1 条 upsert + 1 条 overview，200 节点峰值 ~400 条/秒 ÷ 4 浏览器 = 每连接 100/s，1秒窗口 buffer 足够)。

### 2. 启动集中 diff 后台任务

在 `SharedState::new` 中 `tokio::spawn` 后台任务：
- 订阅 `subscribe_browser_updates()` 脏信号
- 去抖(1秒 interval)
- 每次 dirty tick:
  1. `list_node_summaries` 一次
  2. `diff_node_lists(&last_snapshot, &current)`
  3. 在同一个 registry 读锁窗口内记录当前 `nodes_revision`
  4. 把增量(`NodeUpsert`/`NodeRemoved`)+ `OverviewUpdate` 包装为 `BrowserIncrementalUpdate`
  5. `browser_incremental_tx.send(update)` 广播

**生命周期**：任务持有 `SharedState` weak ref，在最后一个 `SharedState` drop 时自然结束(脏信号 channel 关闭 → task 退出)。

### 3. 改造 `run_browser_session` 订阅增量 channel

删除会话内的：
- `subscribe_browser_updates()` 订阅
- `last_nodes` 快照维护
- `diff_node_lists` 调用
- `push_incremental_updates` 重算

新逻辑：
```rust
let mut incremental_rx = shared.subscribe_browser_incremental();
let mut baseline_revision = send_initial_state(&shared, &mut sender).await?;
loop {
    tokio::select! {
        incoming = receiver.next() => { /* handle client messages */ }
        incremental = incremental_rx.recv() => {
            match incremental {
                Ok(update) if update.revision > baseline_revision => {
                    send_browser_message(&mut sender, &update.message).await?;
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {
                    // 落后 → 重发全量 InitialState
                    baseline_revision = send_initial_state(&shared, &mut sender).await?;
                }
                Err(RecvError::Closed) => return Ok(()),
            }
        }
    }
}
```

去抖 interval 删除(集中任务已去抖)；客户端 Ping/Pong 保持不变。

### 4. `InitialState` 仍由各会话独立发送

连接建立时立即发送全量 `InitialState`(各会话时机不同，无法集中)；后续增量统一从集中任务接收。
`InitialState` 会返回本次全量快照对应的 `nodes_revision` baseline。会话只转发
`revision > baseline` 的增量，丢弃已排队但不晚于 baseline 的旧 diff，避免旧增量在
`InitialState` 后到达并覆盖新快照。`Lagged` 重同步也刷新同一个 baseline。

## 测试验证

**行为不变性**由既有测试保障：
- 12 条 `browser.rs` 单测(diff/分类逻辑未变)
- 7 条集成测试(InitialState、upsert、变化、离线、静默期、ping/pong、未认证拒绝)

**性能验证**(手动，不入 CI)：
- 用负载测试工具建立 10+ 浏览器连接，观察服务端 CPU 与锁竞争指标
- 预期：高并发下 registry RwLock 读锁等待时间显著下降

## 风险与权衡

- **broadcast channel 容量**：256 足够覆盖 1 秒窗口的峰值流量；若超出，慢连接收到 `Lagged` 后重发 `InitialState` 强制重同步(与当前行为一致)
- **InitialState / 增量竞态**：服务端用 `nodes_revision` 作为快照 baseline，而非依赖时间戳或 drain。旧 revision 的排队增量会被丢弃，晚于 baseline 的增量继续转发。
- **内存开销**：每条消息 `Arc` 共享(零拷贝)，总开销 < 当前逐连接克隆 `Vec<NodeListItem>`
- **单点故障**：集中任务 panic → 所有浏览器连接失去增量推送；用 `catch_unwind` 或让任务自然退出(连接超时后客户端重连)

## 后续优化空间

- 若 200+ 节点时 `list_node_summaries` 克隆仍是瓶颈 → 引入 Copy-on-Write `im::Vector` 或 `Arc<Vec>`
- 若增量广播 channel 成为竞争点 → 按连接 shard 到多个 channel(暂无必要，单 channel 可支撑数千订阅者)
