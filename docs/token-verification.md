# Token 验证并发与内存预算

NodeLite 使用 Argon2id 验证 Agent Token。每次冷缓存验证约需要 19 MiB 临时工作内存，
因此 `server.token_verify_max_parallelism` 同时控制重连风暴的内存峰值和排队时间。

默认值为 4，允许范围为 1 到 8。常用配置取舍如下：

| 并发值 | Argon2 理论工作内存 | 适用场景 |
| ---: | ---: | --- |
| 2 | 约 38 MiB | 小内存 VPS，接受更长的批量重连时间 |
| 4 | 约 76 MiB | 默认配置，平衡内存与恢复速度 |
| 8 | 约 152 MiB | 内存充足，需要缩短大规模重连排队 |

配置只影响冷缓存 Argon2 验证。验证结果仍按原有 5 分钟 TTL 缓存；Token 轮换或注册表
版本变化仍会使旧缓存失效，认证结果和缓存安全边界不变。

## 监控

`/metrics` 暴露以下指标：

- `nodelite_token_verify_limit`：当前配置的最大并发数；
- `nodelite_token_verify_active`：正在执行 Argon2 的任务数；
- `nodelite_token_verify_waiting`：正在等待并发许可的请求数；
- `nodelite_token_verify_wait_seconds_total`：所有请求累计等待许可的秒数；
- `nodelite_token_cache_hits_total`：由未过期缓存直接返回的验证次数，包括获取并发许可后的二次检查命中；
- `nodelite_token_cache_misses_total`：实际执行 Argon2 验证的次数；
- `nodelite_token_cache_evictions_total`：缓存达到容量上限后发生的 LRU 驱逐次数。

缓存命中包含成功和失败的验证结果。`evictions_total` 只统计容量驱逐，不统计 TTL 过期
清理或 Token 轮换触发的显式清空。诊断命中率时可比较
`rate(nodelite_token_cache_hits_total[5m])` 与 hits、misses 两者速率之和；如果 miss 较高但
eviction 保持为 0，应优先检查并发冷启动、注册表 revision 变化或 Token 轮换，而不是直接
增大缓存。

如果 `waiting` 长时间大于 0，且主机仍有足够内存，可以逐步提高并发。小内存主机应优先
保持 2 或 4，并结合进程 RSS、cgroup `MemoryCurrent` 和 OOM 日志判断，而不是只看单次
重连耗时。

## 压力测量

仓库内置 200 节点冷缓存验证测试。每个并发档位应在独立测试进程中运行，避免分配器
保留上一档 Argon2 工作区而污染 RSS 基线：

```bash
for parallelism in 2 4 8; do
  NODELITE_TOKEN_VERIFY_PARALLELISM="$parallelism" \
    cargo bench -p nodelite-server --features bench-internals --bench load -- token-budget
done
```

2026-07-10 在 macOS debug 测试进程中的参考结果如下。它用于验证相对预算和配置趋势，
不是 Linux release 部署的容量承诺：

| 并发值 | 整批耗时 | 单次认证 p95 | RSS 峰值 | 相对基线增量 | active 峰值 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 32.20 s | 30.61 s | 57.73 MiB | 21.61 MiB | 2 |
| 4 | 16.13 s | 15.48 s | 97.52 MiB | 59.66 MiB | 4 |
| 8 | 10.49 s | 10.06 s | 172.69 MiB | 136.66 MiB | 8 |

测试要求 RSS 增量不超过 `并发值 × 19 MiB + 16 MiB`。额外 16 MiB 只用于容纳测试运行时、
任务和采样噪声，不应被当作生产环境可额外占用的固定预算。


## 重连缓存命中率复核（#306）

2026-09-14，Linux aarch64 隔离 VM（4 vCPU、6 GiB RAM），Rust 1.98.1 `bench` 优化构建，缓存容量 512、验证并发采用当前默认 4。运行上述 `reconnect` 场景：每组节点执行四次连接，第一轮冷缓存，之后三轮使用相同凭证；每次连接仍发送指标并并发读取 Overview/Nodes API。

| 节点数 | 连接次数 | 真实 hits | 实际 Argon2 misses | LRU evictions | revision 变化 | 命中率 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 20 | 80 | 60 | 20 | 0 | 0 | 75% |
| 50 | 200 | 150 | 50 | 0 | 0 | 75% |
| 100 | 400 | 300 | 100 | 0 | 0 | 75% |
| 200 | 800 | 600 | 200 | 0 | 0 | 75% |

200 节点每轮的关联结果：

| 轮次 | hits / misses | 客户端 auth wait p50 / p95 | auth wait < 2ms 占比 |
| ---: | ---: | ---: | ---: |
| 1（冷） | 0 / 200 | 535.64 / 1219.00 ms | 0% |
| 2（热） | 200 / 0 | 4.17 / 6.24 ms | 16% |
| 3（热） | 200 / 0 | 3.65 / 7.02 ms | 14.5% |
| 4（热） | 200 / 0 | 3.86 / 7.02 ms | 5.5% |

旧报告把 `auth_wait < 2ms` 当作命中；这个客户端计时还包含任务调度、消息收发和认证后的工作。本次相同方法只会得到 **9%**（72/800），而真实计数为 **75%**。因此旧报告的 33% 不能证明缓存未命中或 LRU 驱逐，其“仍在逐出”的结论应撤回。当前纯重连场景未发现 revision 抖动或容量不足，无须再次扩大缓存。配置重载、Token 轮换、TTL 过期及同时验证同一冷 Token 属于不同负载，仍由安全失效逻辑和对应单元测试约束。

`TOKEN_CACHE_CYCLE_RESULT` 输出每轮计数差值、revision 与整批连接耗时；`AGENT_TIMING` 输出各连接的客户端时间。汇总以 `TOKEN_CACHE_RESULT` 为准，harness 会拒绝纯重连中的 revision 变化、容量驱逐或不超过 60% 的命中率。这里的性能数字只用于复现方法，不与旧机器/旧并发配置的耗时直接比较。
