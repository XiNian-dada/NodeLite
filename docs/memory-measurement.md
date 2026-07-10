# Memory measurement methodology

NodeLite does not use a single memory number as the release baseline. Process
RSS, proportional set size, anonymous memory, and a systemd cgroup answer
different questions and must be recorded together.

All byte totals in benchmark reports use binary units: 1 KiB = 1024 bytes and
1 MiB = 1024 KiB.

## Required report context

Every result must include:

- NodeLite commit or release tag, release target, allocator, and build command;
- operating system, kernel, architecture, vCPU count, and total RAM;
- Server or Agent uptime and the number of established Agent/browser sessions;
- report interval and whether the process is disconnected or authenticated;
- whether dashboard and History pages were opened before collection;
- History/Audit database and WAL sizes;
- whether the sample represents a cold process or a warmed workload;
- swap activity during the sample, not only the amount of cold data in swap.

For fleet tests, report idle, 5, 200, and 1000 Agents separately. For the
Agent, report disconnected idle, authenticated idle, 1-second reporting, and a
large-mount workload separately.

## Reference production sample

The following 2026-07-10 sample is context, not a portable release gate. The
Linux Server had 5 Agents, 4 days of uptime, an approximately 31 MiB History DB,
a 4.1 MiB History WAL, a 572 KiB Audit DB, and no sustained swap I/O:

| Metric | Observed |
|---|---:|
| systemd `MemoryCurrent` | 38.45 MiB |
| process RSS / PSS | 15.6 MiB |
| `RssAnon` / `Pss_Anon` | 8.3-8.5 MiB |
| `RssFile` / `Pss_File` | 7.3 MiB |
| cgroup `file` | 28.55 MiB |
| cgroup `file_mapped` | 7.29 MiB |
| cgroup `file_dirty` / `file_writeback` | 24 KiB / 0 |
| cgroup `sock` | 12 KiB |
| `SwapPss` | 6.14 MiB |

The roughly 21.3 MiB gap between cgroup `file` and process `RssFile` was
reclaimable filesystem page cache, consistent with the SQLite DB/WAL working
set. `vmstat 1 5` reported `si=0` and `so=0`, so the cold swapped pages weren't
thrashing. This sample demonstrates why `MemoryCurrent=38.45MiB` must not be
reported as a 38.45 MiB Rust heap.

An Ubuntu Agent from the same observation window reported approximately 7.3
MiB current, 7.6 MiB peak, and 4 tasks after 4 days. It showed a stable runtime
baseline rather than continuing growth, but the sample didn't include PSS and
must not be compared directly with a different build target or host.

A separate controlled Linux benchmark for the Agent current-thread runtime
change used static aarch64 musl release binaries, an external mock WebSocket
server, successful authentication, and 1-second reports. Each result was the
median of three runs on the same host:

| Scenario | Baseline PSS | Changed PSS | Threads before/after |
|---|---:|---:|---:|
| 1 vCPU authenticated | 2,788 KiB | 2,660 KiB | 3 / 2 |
| 2 vCPU authenticated | 2,812 KiB | 2,660 KiB | 4 / 2 |
| 8 vCPU authenticated | 2,852 KiB | 2,660 KiB | 10 / 2 |
| 2 vCPU disconnected | 2,604 KiB | 2,440 KiB | not recorded / 2 |

The 1-vCPU authenticated `VmRSS` changed from 2,688 KiB to 2,560 KiB. With
500 bind mounts, the changed build reached 2,664 KiB PSS and a worst observed
`VmHWM` of 2,688 KiB, within 256 KiB of the ordinary-idle median. These musl
results demonstrate a reproducible comparison protocol; they do not replace a
glibc/systemd measurement on the deployment host.

## Linux process metrics

Set `service` to `nodelite-server.service` or `nodelite-agent.service`:

```bash
service=nodelite-server.service
pid=$(systemctl show "$service" -p MainPID --value)

grep -E \
  '^(VmRSS|RssAnon|RssFile|RssShmem|VmData|VmStk|Threads)' \
  "/proc/$pid/status"

grep -E \
  '^(Rss|Pss|Pss_Anon|Pss_File|Private|Shared|Anonymous|Swap|SwapPss)' \
  "/proc/$pid/smaps_rollup"
```

Interpretation:

- `VmRSS`/`Rss` is the process resident set and includes private and mapped
  file-backed pages. Shared pages are counted in full.
- `Pss` divides shared pages proportionally and is the preferred process-level
  comparison when it is available.
- `RssAnon`/`Pss_Anon` is the useful first approximation for heap, stacks,
  runtime state, and private SQLite page caches. It is not a perfect Rust heap
  measurement.
- `RssFile`/`Pss_File` includes executable, shared-library, mmap, and other
  file-backed resident pages.
- `SwapPss` can contain cold pages without indicating active memory pressure.
  Check `vmstat` before describing it as swap thrashing.
- `Threads` is part of the Agent baseline because Tokio runtime selection can
  change both stack reservation and scheduling overhead.

## systemd cgroup metrics

```bash
service=nodelite-server.service
cgroup="/sys/fs/cgroup/system.slice/$service"

systemctl show "$service" \
  -p MemoryCurrent -p MemoryPeak -p TasksCurrent

grep -E \
  '^(anon|file|file_mapped|file_dirty|file_writeback|kernel|sock|slab|pagetables|active_file|inactive_file) ' \
  "$cgroup/memory.stat"
```

`MemoryCurrent` is the total memory charged to the service cgroup. It includes
process anonymous memory, mapped files, kernel allocations, socket memory, and
filesystem page cache created by database and log I/O. It is the correct number
for a systemd resource budget, but it is not a Rust heap or leak metric.

The cgroup `file` counter can be much larger than process `RssFile`: SQLite DB
and WAL pages can remain in the reclaimable filesystem cache after they are no
longer mapped into the process. Record `file_mapped`, `file_dirty`, and
`file_writeback` before attributing the difference to an application leak.

Do not run global `drop_caches`, switch to direct I/O, or continuously clear
caches merely to reduce `systemctl status` output. Those actions affect the
whole host and can make History latency worse. If a cold-cache experiment is
required, run it on an isolated disposable host and label it explicitly.

## Database and swap context

```bash
data_dir=/var/lib/nodelite
pid=$(systemctl show nodelite-server.service -p MainPID --value)

du -h "$data_dir"
find "$data_dir" -maxdepth 1 -type f \
  \( -name '*.sqlite3' -o -name '*.sqlite3-wal' -o -name '*.sqlite3-shm' \) \
  -exec ls -lh {} +
lsof -p "$pid" | grep -E 'sqlite|wal|shm|snapshot|server.json'
vmstat 1 5
```

Record History DB/WAL sizes for both cold and warmed History scenarios. In
`vmstat`, sustained non-zero `si`/`so` indicates active swap I/O; a non-zero
`SwapPss` with `si=0` and `so=0` does not by itself show thrashing.

## Scenario protocol

Use release binaries on the same host for before/after comparisons.

1. Start the service and wait for initialization to settle.
2. Record the cold idle sample before opening dashboard or History pages.
3. Connect the declared number of external Agents and wait for authentication.
4. Record authenticated idle after at least five stable sample intervals.
5. Open the dashboard and record the warmed dashboard sample.
6. Query representative History windows, record the warmed History sample,
   and include DB/WAL sizes and query p95.
7. Keep sampling long enough to observe `MemoryPeak` and confirm whether RSS
   returns to a stable plateau.

External-client fleet measurements must read the Server PID only. The built-in
loopback load tests run fake clients and Server in one process; their latency
and throughput are useful, but their RSS is not a Server-only memory baseline.

## Reporting template

| Field | Value |
|---|---|
| Version / target | |
| Host / kernel / vCPU / RAM | |
| Uptime | |
| Scenario and Agent/browser count | |
| Report interval | |
| Dashboard/History warmed | |
| `MemoryCurrent` / `MemoryPeak` | |
| RSS / PSS | |
| `RssAnon` / `RssFile` | |
| cgroup `anon` / `file` / `file_mapped` | |
| `SwapPss`; `vmstat si/so` | |
| Threads / TasksCurrent | |
| History DB / WAL / SHM | |
| Audit DB / WAL / SHM | |
| Query/API p95 | |

Release decisions should compare like-for-like scenarios and define budgets
per scenario. Historical `<15MB Server` and `<2MB Agent` statements did not
specify a platform or metric and are therefore not release gates.
