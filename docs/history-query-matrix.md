# History query resource matrix

The history-only matrix isolates SQLite reads from Agent WebSocket authentication. It seeds 1,000
nodes with 480 points each, then runs these cases in separate child processes:

- a legacy-equivalent baseline with no effective application concurrency limit and SQLite's
  default read cache (approximately 2 MiB per connection);
- query concurrency: `2`, `4`, `8`;
- SQLite read cache: `256`, `512`, `1024` KiB per connection.

Run it with an optimized build:

```bash
cargo test -p nodelite-server --release load_test_history_query_matrix_scores -- --ignored --nocapture
```

The parent process seeds one shared database and launches each case in a fresh copy of the test
executable. Each child captures idle memory, samples memory every 10 ms while queries run, and
returns the observed peak. `HISTORY_QUERY_MATRIX_RESULT` reports p50, p95, max latency, peak delta
from that child's idle value, and delta from the legacy baseline. Linux reports RSS, PSS and
`RssAnon`. macOS only exposes RSS through the portable benchmark probe, so its PSS and
anonymous-memory fields are printed as `unavailable` and must not be used for memory-limit
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

The query-result cache's `estimated_bytes` budget includes String/Vec allocation metadata, LRU/hash
entry overhead, and a 12.5% safety margin for allocator size classes and alignment. It remains a
portable conservative estimate rather than an allocator-reported exact heap measurement; Linux
PSS/RssAnon is still the final production validation source.

## 2026-07-10 macOS result

Environment: Apple M1 Pro, 32 GiB RAM, macOS Darwin 25.5.0, Rust 1.93.1, release profile. The table
is the final verification run after warming the SQLite covering index. Every row is a new process.
RSS is not a substitute for Linux PSS/RssAnon.

| Case | Concurrency | Read cache KiB | p95 ms | p95 vs baseline ms | Peak RSS bytes | Peak RSS delta from idle | Delta vs baseline |
|---|---:|---:|---:|---:|---:|---:|---:|
| Legacy baseline | effectively unbounded | SQLite default | 431.30 | 0.00 | 178,110,464 | 168,230,912 | 0 |
| Configured | 2 | 256 | 511.23 | +79.93 | 22,495,232 | 12,599,296 | -155,631,616 |
| Configured | 2 | 512 | 488.73 | +57.43 | 22,970,368 | 13,074,432 | -155,156,480 |
| Configured | 2 | 1024 | 489.42 | +58.12 | 22,708,224 | 12,828,672 | -155,402,240 |
| Configured | 4 | 256 | 379.32 | -51.97 | 24,707,072 | 14,860,288 | -153,370,624 |
| Configured | 4 | 512 | 371.69 | -59.61 | 24,231,936 | 14,336,000 | -153,894,912 |
| Configured | 4 | 1024 | 374.16 | -57.14 | 24,199,168 | 14,303,232 | -153,927,680 |
| Configured | 8 | 256 | 461.62 | +30.32 | 26,230,784 | 16,334,848 | -151,896,064 |
| Configured | 8 | 512 | 449.06 | +17.77 | 26,345,472 | 16,449,536 | -151,781,376 |
| Configured | 8 | 1024 | 461.68 | +30.38 | 25,411,584 | 15,532,032 | -152,698,880 |

The default `4 / 512 KiB` cut peak RSS growth by 153,894,912 bytes (91.5%) versus the
legacy-equivalent baseline while improving p95 by 59.61 ms in the final run. Across two verification
runs, baseline peak growth stayed at 160-162 MiB while the default stayed at 13.7-14.3 MiB; p95 varied
from 431-469 ms for the baseline and 372-383 ms for the default. The three concurrency-4 cases were
the fastest group in both runs. `512 KiB` avoids choosing the largest per-connection cache based on
small differences. Linux peak PSS/RssAnon sampling remains required before treating these macOS RSS
results as production memory limits.
