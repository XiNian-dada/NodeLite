# History query resource matrix

The history-only matrix isolates SQLite reads from Agent WebSocket authentication. It seeds 1,000
nodes with 480 points each, then runs every combination of:

- query concurrency: `2`, `4`, `8`;
- SQLite read cache: `256`, `512`, `1024` KiB per connection.

Run it with an optimized build:

```bash
cargo test -p nodelite-server --release load_test_history_query_matrix_scores -- --ignored --nocapture
```

Each `HISTORY_QUERY_MATRIX_RESULT` line reports p50, p95, max latency and process memory. Linux
reports RSS, PSS and `RssAnon`. macOS only exposes RSS through the portable benchmark probe, so its
PSS and anonymous-memory fields are printed as `unavailable` and must not be used for memory-limit
selection.

For a Linux production process, capture the memory split separately while history requests are in
flight:

```bash
pid=$(systemctl show nodelite-server.service -p MainPID --value)
grep -E '^(VmRSS|RssAnon|RssFile|RssShmem|Swap|Threads)' /proc/$pid/status
grep -E '^(Rss|Pss|Pss_Anon|Pss_File|Anonymous|Swap|SwapPss)' /proc/$pid/smaps_rollup
grep -E '^(anon|file|kernel|sock|slab|pagetables|active_file|inactive_file) ' \
  /sys/fs/cgroup/system.slice/nodelite-server.service/memory.stat
```

The cgroup `file` value is Linux filesystem page cache. It is reclaimable kernel memory and is not
the same resource as SQLite's per-connection private page cache controlled by
`history_read_cache_kib`.

## 2026-07-10 macOS result

Environment: Apple M1 Pro, 32 GiB RAM, macOS Darwin 25.5.0, Rust 1.93.1, release profile. This was a
single matrix run after warming the SQLite covering index. RSS is the cumulative test-process RSS
after each case and is not a substitute for Linux PSS/RssAnon.

| Concurrency | Read cache KiB | p50 ms | p95 ms | Max ms | RSS bytes |
|---:|---:|---:|---:|---:|---:|
| 2 | 256 | 255.38 | 497.10 | 524.19 | 28,409,856 |
| 2 | 512 | 295.11 | 566.92 | 593.49 | 28,835,840 |
| 2 | 1024 | 286.88 | 584.32 | 619.48 | 29,294,592 |
| 4 | 256 | 255.45 | 436.27 | 456.58 | 31,981,568 |
| 4 | 512 | 195.78 | 366.65 | 389.32 | 33,652,736 |
| 4 | 1024 | 190.20 | 379.77 | 399.13 | 33,669,120 |
| 8 | 256 | 307.99 | 665.94 | 692.77 | 34,373,632 |
| 8 | 512 | 290.17 | 572.56 | 598.21 | 34,390,016 |
| 8 | 1024 | 471.00 | 710.44 | 731.51 | 34,832,384 |

The default `4 / 512 KiB` combination had the lowest measured p95 in this run. Concurrency `8`
increased contention and was slower for every cache size; concurrency `2` reduced in-flight memory
but increased queueing. Linux PSS/RssAnon sampling remains required before changing the production
default based on memory rather than latency.
