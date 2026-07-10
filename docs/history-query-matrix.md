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
| 2 | 256 | 245.53 | 467.64 | 491.81 | 26,394,624 |
| 2 | 512 | 254.26 | 477.78 | 502.01 | 26,689,536 |
| 2 | 1024 | 241.21 | 461.45 | 486.40 | 26,755,072 |
| 4 | 256 | 189.33 | 356.24 | 374.00 | 29,638,656 |
| 4 | 512 | 192.54 | 362.75 | 382.74 | 29,736,960 |
| 4 | 1024 | 189.85 | 354.79 | 374.31 | 29,769,728 |
| 8 | 256 | 247.00 | 468.58 | 490.87 | 32,079,872 |
| 8 | 512 | 247.72 | 465.48 | 488.64 | 32,768,000 |
| 8 | 1024 | 248.17 | 468.25 | 491.38 | 32,768,000 |

All concurrency-4 cases were within 2.3% p95 of each other and materially faster than concurrency
2 or 8. The default `4 / 512 KiB` stays in the middle of that stable latency band without doubling
the per-connection cache to 1024 KiB. Linux PSS/RssAnon sampling and repeated runs remain required
before changing the production default based on small single-run differences.
