---
title: 性能基准
nav_order: 9
---

# 性能基准

下面的数据来自仓库中的压测基准（`nodelite-server/benches`），用来说明资源占用的量级，不是对任意环境的保证。

**测试环境**：Intel Core i5-13600K，16 GB 内存，Ubuntu 22.04（Linux 5.15，x86_64），`release` 构建并开启 LTO，本机回环网络，全链路 TLS/WSS 认证。

## 节点数量扩展

| 节点数 | 全部建立连接 | 稳态吞吐（指标/秒） | Overview API p95 |
| ---: | ---: | ---: | ---: |
| 20 | 84.7 ms | 11,273 | 26.03 ms |
| 50 | 210.5 ms | 29,255 | 30.76 ms |
| 100 | 375.7 ms | 18,662 | 27.21 ms |
| 200 | 757.9 ms | 110,970 | 24.61 ms |

## 大规模集群

| 节点数 | 全部接入 | 稳态吞吐（指标/秒） | API p95（Overview / Nodes / Prometheus） | Server RSS | 历史写入丢弃 |
| ---: | ---: | ---: | --- | ---: | ---: |
| 500 | 2.33 s | 46,603 | 36.08 / 312.85 / 44.75 ms | ~488 MB | 0 |
| 1,000 | 3.75 s | 67,480 | 35.37 / 38.14 / 85.24 ms | ~514 MB | 0 |

## 重连风暴

200 个节点连续 4 轮断线重连（共 800 次）：Token 校验缓存命中率 75%（后 3 轮全部命中），命中时连接耗时约 3.9 ms，连接延迟中位数 4.2 ms。

## 多人同时查看面板

1,000 节点下 20 个面板并发刷新：Overview 与节点列表的视图缓存命中率分别为 96% 和 99%，吞吐保持在 72,211 指标/秒，历史写入无丢弃。

## 长时间运行

在一台 Intel Xeon Gold 6138 生产服务器上：Agent 连续运行 3 个月，常驻内存约 1.3 MB；Server 带 6 个节点运行半个月，内存稳定在约 8.7 MB。

## 自己运行

```bash
cargo bench -p nodelite-server --features bench-internals --bench load -- scaling
cargo bench -p nodelite-server --features bench-internals --bench load -- reconnect
# 以下需要 ulimit -n >= 65536
cargo bench -p nodelite-server --features bench-internals --bench load -- large-fleet
cargo bench -p nodelite-server --features bench-internals --bench load -- dashboard
cargo bench -p nodelite-server --features bench-internals --bench load -- history-pressure
```

内存的测量口径（RSS、PSS、cgroup）见[内存测量](memory-measurement.md)。
