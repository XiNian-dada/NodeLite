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
- `nodelite_token_verify_wait_seconds_total`：所有请求累计等待许可的秒数。

如果 `waiting` 长时间大于 0，且主机仍有足够内存，可以逐步提高并发。小内存主机应优先
保持 2 或 4，并结合进程 RSS、cgroup `MemoryCurrent` 和 OOM 日志判断，而不是只看单次
重连耗时。

## 压力测量

仓库内置 200 节点冷缓存验证测试。每个并发档位应在独立测试进程中运行，避免分配器
保留上一档 Argon2 工作区而污染 RSS 基线：

```bash
for parallelism in 2 4 8; do
  NODELITE_TOKEN_VERIFY_PARALLELISM="$parallelism" \
    cargo test -p nodelite-server load_test_token_verify_storm_budget \
      -- --ignored --nocapture
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
