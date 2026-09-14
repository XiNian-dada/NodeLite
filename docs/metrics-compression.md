# Metrics 传输压缩

RC1 Agent 在 Hello 中声明 `supports_metrics_zlib = true`。Server 只有在验证凭证并接受该能力后，才在 `authenticated` 通知中返回 `code = "metrics_zlib_v1"`；Agent 收到这个确认后才压缩 Metrics。每次重连都会重新协商。

旧 Server 忽略新增 Hello 字段，返回旧认证通知，新 Agent 继续发送 JSON；新 Server 继续接收未声明能力的旧 Agent。wire protocol 版本范围仍为 1–3。Identity 已经仅在 Hello 发送，此次未改变快照字段语义，也未引入依赖前帧的增量状态。

当前 axum/tungstenite 栈不支持 `permessage-deflate`。因此 #260 的带宽优化使用应用层的独立 zlib 帧：二进制内容是 ASCII `NLMZ1` 加一条 zlib 流，解压后为 `MetricsMessage` 的 JSON。认证凭证、日志和其他控制消息仍走原有文本帧；压缩字典不会跨帧或跨节点共享。

Server 在认证并协商前拒绝二进制帧。每帧的压缩字节和解压 JSON 都受 `server.max_message_bytes` 约束，解压另有 1 MiB 硬上限；截断、损坏、尾随内容和多个拼接流均被拒绝。解压后的快照还经过现有指标校验。

## 测量

2026-09-14，Linux aarch64、Rust 1.98.1 优化构建，每种布局序列化 1000 条变化中的真实 Metrics，逐条压缩、解压并校验完全相等。下表计入客户端 WebSocket mask 和帧头，不含 TCP/IP/TLS、握手及其他消息：

| 挂载点 | JSON 帧总字节 | 压缩帧总字节 | 减少 | 每条压缩平均耗时 |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 633261 | 353052 | 44.25% | 22.21 µs |
| 8 | 1827255 | 431412 | 76.39% | 23.08 µs |
| 16 | 3179255 | 502456 | 84.20% | 34.66 µs |
| 64 | 11291258 | 853803 | 92.44% | 72.90 µs |

Issue 中约 2 KiB 快照的 60% 目标在相近的 8 挂载点负载上达到。更小快照受独立字典的固定成本影响，收益较低，不能保证每一种快照都减少 60%。测试保留小快照结果，并对其要求字节下降，对约 2 KiB 及以上场景要求下降超过 60%。

```bash
cargo bench -p nodelite-server --features bench-internals --bench load -- wire-bandwidth
```

真实 WebSocket 回归另行覆盖 Server 协商、压缩/普通 JSON 数据入库路径、未协商拒绝和解压超限；Agent 集成测试使用真实采集器验证旧通知回退和新通知启用压缩。
