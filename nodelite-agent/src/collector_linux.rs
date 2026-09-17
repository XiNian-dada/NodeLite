//! Linux 主机指标采集器:读取 `/proc`、`statvfs` 等内核接口,
//! 把原始数据归并成 `nodelite-proto` 中定义的快照与身份结构。

use std::fs;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use chrono::{Duration, Utc};
use nodelite_proto::{
    AgentConfig, LoadAverage, MemoryUsage, NetworkCounters, NodeIdentity, NodeSnapshot,
};

#[path = "collector_linux/disks.rs"]
mod disks;

#[cfg(test)]
#[path = "collector_linux/tests.rs"]
mod tests;

use self::disks::{StatvfsFn, collect_disks, real_statvfs};
use super::shared::{
    CpuSample, NetworkRateBaselines, NetworkSample, NetworkTotals, compute_cpu_usage,
    compute_network_metrics,
};

/// 采集器状态:为了计算 CPU/网络的"差分速率",需要保留上一次的采样值。
pub struct HostCollector {
    sys_root: std::path::PathBuf,
    previous_cpu: Option<CpuSample>,
    previous_network: Option<NetworkSample>,
    network_rate_baselines: NetworkRateBaselines,
    /// 磁盘容量探测函数。默认是真实的 `statvfs` 系统调用,测试可通过
    /// [`HostCollector::with_statvfs`] 注入桩实现。
    statvfs: StatvfsFn,
}

pub fn new_collector() -> HostCollector {
    HostCollector {
        sys_root: std::path::PathBuf::from("/"),
        previous_cpu: None,
        previous_network: None,
        network_rate_baselines: NetworkRateBaselines::default(),
        statvfs: real_statvfs,
    }
}

impl HostCollector {
    #[cfg(test)]
    fn new_with_root(sys_root: std::path::PathBuf) -> Self {
        Self {
            sys_root,
            previous_cpu: None,
            previous_network: None,
            network_rate_baselines: NetworkRateBaselines::default(),
            statvfs: real_statvfs,
        }
    }

    /// 注入磁盘容量探测桩,使快照采集完全脱离真实文件系统。仅供测试使用。
    #[cfg(test)]
    fn with_statvfs(mut self, statvfs: StatvfsFn) -> Self {
        self.statvfs = statvfs;
        self
    }

    /// 组装节点身份。`agent_version` 来源于编译期注入,运行期固定不变。
    pub fn collect_identity(
        &self,
        config: &AgentConfig,
        agent_version: &str,
    ) -> Result<NodeIdentity> {
        let uptime_path = self.sys_root.join("proc/uptime");
        let uptime_secs = read_uptime(&uptime_path)?;
        // 由当前时刻反推启动时间,在 i64 转换溢出时退化为 i64::MAX 防止 panic。
        let boot_time =
            Utc::now() - Duration::seconds(i64::try_from(uptime_secs).unwrap_or(i64::MAX));

        let hostname_path = self.sys_root.join("proc/sys/kernel/hostname");
        let os_release_path = self.sys_root.join("etc/os-release");
        let osrelease_path = self.sys_root.join("proc/sys/kernel/osrelease");
        let cpuinfo_path = self.sys_root.join("proc/cpuinfo");

        Ok(NodeIdentity {
            node_id: config.node_id.clone(),
            node_label: config.node_label.clone(),
            hostname: config
                .hostname_override
                .clone()
                .unwrap_or(read_hostname(&hostname_path)?),
            os: read_os_name(&os_release_path).unwrap_or_else(|_| "linux".to_string()),
            kernel_version: read_trimmed(&osrelease_path).ok(),
            cpu_model: read_cpu_model(&cpuinfo_path).ok(),
            cpu_cores: count_cpu_cores(&cpuinfo_path).unwrap_or(1),
            agent_version: agent_version.to_string(),
            boot_time: Some(boot_time),
            tags: config.tags.clone(),
        })
    }

    /// 采集一张完整快照。
    ///
    /// 首次调用时由于没有"上一次"的数据,`cpu_usage_percent` 与网络速率
    /// 都会返回 `None`,这是符合预期的初始状态。
    pub fn collect_snapshot(&mut self, ignored_filesystems: &[String]) -> Result<NodeSnapshot> {
        let stat_path = self.sys_root.join("proc/stat");
        let cpu_sample =
            parse_cpu_sample(&fs::read_to_string(&stat_path).context("read /proc/stat")?)?;
        let cpu_usage_percent = self
            .previous_cpu
            .map(|previous| compute_cpu_usage(previous, cpu_sample));
        self.previous_cpu = Some(cpu_sample);

        let dev_path = self.sys_root.join("proc/net/dev");
        let network_totals =
            parse_network_totals(&fs::read_to_string(&dev_path).context("read /proc/net/dev")?)?;
        let observed_at = Instant::now();
        let network_metrics = if let Some(previous) = self.previous_network {
            compute_network_metrics(previous, observed_at, network_totals)
        } else {
            Default::default()
        };
        self.previous_network = Some(NetworkSample {
            observed_at,
            rx_bytes: network_totals.rx_bytes,
            tx_bytes: network_totals.tx_bytes,
            rx_packets: network_totals.rx_packets,
            tx_packets: network_totals.tx_packets,
            rx_dropped_packets: network_totals.rx_dropped_packets,
            tx_dropped_packets: network_totals.tx_dropped_packets,
        });
        super::log_network_rate_anomalies(self.network_rate_baselines.observe(
            network_metrics.rx_bytes_per_sec,
            network_metrics.tx_bytes_per_sec,
        ));

        let loadavg_path = self.sys_root.join("proc/loadavg");
        let load =
            parse_load_average(&fs::read_to_string(&loadavg_path).context("read /proc/loadavg")?)?;
        let meminfo_path = self.sys_root.join("proc/meminfo");
        let memory =
            parse_memory_usage(&fs::read_to_string(&meminfo_path).context("read /proc/meminfo")?)?;
        let uptime_path = self.sys_root.join("proc/uptime");
        let uptime_secs = read_uptime(&uptime_path)?;
        let mounts_path = self.sys_root.join("proc/mounts");
        let disks = collect_disks(&mounts_path, self.statvfs, ignored_filesystems)?;

        Ok(NodeSnapshot {
            collected_at: Utc::now(),
            cpu_usage_percent,
            load,
            memory,
            uptime_secs,
            disks,
            network: NetworkCounters {
                total_rx_bytes: network_totals.rx_bytes,
                total_tx_bytes: network_totals.tx_bytes,
                rx_bytes_per_sec: network_metrics.rx_bytes_per_sec,
                tx_bytes_per_sec: network_metrics.tx_bytes_per_sec,
                packet_loss_percent: network_metrics.packet_loss_percent,
            },
        })
    }
}

/// 读取文件文本并去除首尾空白。
fn read_trimmed(path: &std::path::Path) -> Result<String> {
    Ok(fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?
        .trim()
        .to_string())
}

fn read_hostname(path: &std::path::Path) -> Result<String> {
    read_trimmed(path)
}

/// 解析 `/etc/os-release`,优先返回 `PRETTY_NAME`,缺失时退化到 `NAME`。
fn read_os_name(path: &std::path::Path) -> Result<String> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
            return Ok(strip_quotes(value));
        }
        if let Some(value) = line.strip_prefix("NAME=") {
            return Ok(strip_quotes(value));
        }
    }
    Err(anyhow!("NAME not found in {}", path.display()))
}

fn strip_quotes(value: &str) -> String {
    value.trim_matches('"').to_string()
}

/// 从 `/proc/cpuinfo` 中提取第一处 `model name`。
fn read_cpu_model(path: &std::path::Path) -> Result<String> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("model name\t: ") {
            return Ok(value.trim().to_string());
        }
    }
    Err(anyhow!("model name not found in {}", path.display()))
}

/// 通过统计 `processor` 行的数量得到逻辑核心数;至少返回 1。
fn count_cpu_cores(path: &std::path::Path) -> Result<u32> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let count = content
        .lines()
        .filter(|line| line.starts_with("processor\t:"))
        .count();
    Ok(u32::try_from(count).unwrap_or(u32::MAX).max(1))
}

/// 读取 `/proc/uptime` 的整数秒部分。
fn read_uptime(path: &std::path::Path) -> Result<u64> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let raw = content
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("missing uptime field in {}", path.display()))?;
    let seconds = raw
        .split('.')
        .next()
        .ok_or_else(|| anyhow!("invalid uptime field in {}", path.display()))?;
    seconds
        .parse::<u64>()
        .with_context(|| format!("invalid uptime value in {}", path.display()))
}

/// 解析 `/proc/stat` 中的 `cpu ` 聚合行。
///
/// 字段顺序:user / nice / system / idle / iowait / ...
/// 这里我们只关心 `total = 全部之和`,以及 `idle = idle + iowait`。
fn parse_cpu_sample(content: &str) -> Result<CpuSample> {
    let line = content
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or_else(|| anyhow!("missing aggregate cpu line"))?;
    let mut total = 0_u64;
    let mut idle = 0_u64;
    let mut counter_count = 0_usize;
    for (index, raw_value) in line.split_whitespace().skip(1).enumerate() {
        let value = raw_value.parse::<u64>().context("invalid cpu counter")?;
        total = total.saturating_add(value);
        if index == 3 || index == 4 {
            idle = idle.saturating_add(value);
        }
        counter_count += 1;
    }
    if counter_count < 5 {
        return Err(anyhow!("expected at least 5 cpu counters"));
    }
    Ok(CpuSample { total, idle })
}

/// 解析 `/proc/loadavg` 的前三个字段(1/5/15 分钟平均负载)。
fn parse_load_average(content: &str) -> Result<LoadAverage> {
    let mut fields = content.split_whitespace();
    let one = parse_next_load_field(&mut fields, "1m")?;
    let five = parse_next_load_field(&mut fields, "5m")?;
    let fifteen = parse_next_load_field(&mut fields, "15m")?;
    Ok(LoadAverage { one, five, fifteen })
}

fn parse_next_load_field<'a>(
    fields: &mut impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<f64> {
    fields
        .next()
        .ok_or_else(|| anyhow!("expected 3 load average values"))?
        .parse::<f64>()
        .with_context(|| format!("invalid {label} load average"))
}

/// 解析 `/proc/meminfo`,把字段单位从 KB 转换为字节。
///
/// `MemAvailable` 若缺失(老内核),则用 `MemFree + Buffers + Cached` 兜底。
fn parse_memory_usage(content: &str) -> Result<MemoryUsage> {
    let mut mem_total_bytes = None;
    let mut mem_available_bytes = None;
    let mut mem_free_bytes = None;
    let mut buffers_bytes = None;
    let mut cached_bytes = None;
    let mut swap_total_bytes = None;
    let mut swap_free_bytes = None;

    for line in content.lines() {
        let Some((key, raw_value)) = line.split_once(':') else {
            continue;
        };
        if !matches!(
            key,
            "MemTotal"
                | "MemAvailable"
                | "MemFree"
                | "Buffers"
                | "Cached"
                | "SwapTotal"
                | "SwapFree"
        ) {
            continue;
        }
        let kilobytes = raw_value
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow!("missing meminfo value for {key}"))?
            .parse::<u64>()
            .with_context(|| format!("invalid meminfo value for {key}"))?;
        let bytes = kilobytes.saturating_mul(1024);
        match key {
            "MemTotal" => mem_total_bytes = Some(bytes),
            "MemAvailable" => mem_available_bytes = Some(bytes),
            "MemFree" => mem_free_bytes = Some(bytes),
            "Buffers" => buffers_bytes = Some(bytes),
            "Cached" => cached_bytes = Some(bytes),
            "SwapTotal" => swap_total_bytes = Some(bytes),
            "SwapFree" => swap_free_bytes = Some(bytes),
            _ => {}
        }
    }

    let total_bytes =
        mem_total_bytes.ok_or_else(|| anyhow!("MemTotal missing from /proc/meminfo"))?;
    let available_bytes = mem_available_bytes
        .or_else(|| {
            Some(
                mem_free_bytes?
                    .saturating_add(buffers_bytes.unwrap_or(0))
                    .saturating_add(cached_bytes.unwrap_or(0)),
            )
        })
        .ok_or_else(|| anyhow!("unable to infer available memory"))?;
    let used_bytes = total_bytes.saturating_sub(available_bytes);
    let swap_total_bytes = swap_total_bytes.unwrap_or(0);
    let swap_free_bytes = swap_free_bytes.unwrap_or(0);

    Ok(MemoryUsage {
        total_bytes,
        used_bytes,
        available_bytes,
        swap_total_bytes,
        swap_used_bytes: swap_total_bytes.saturating_sub(swap_free_bytes),
    })
}

/// 汇总 `/proc/net/dev` 中所有物理网卡的累计收发字节、包数与丢包数。
/// 跳过 `lo`(回环口),避免本机通信被统计为外部流量。
fn parse_network_totals(content: &str) -> Result<NetworkTotals> {
    let mut rx_bytes = 0_u64;
    let mut tx_bytes = 0_u64;
    let mut rx_packets = 0_u64;
    let mut tx_packets = 0_u64;
    let mut rx_dropped_packets = 0_u64;
    let mut tx_dropped_packets = 0_u64;

    for line in content.lines().skip(2) {
        let Some((iface, counters)) = line.split_once(':') else {
            continue;
        };
        if iface.trim() == "lo" {
            continue;
        }
        let iface_totals = parse_network_line_counters(counters, iface.trim())?;
        rx_bytes = rx_bytes.saturating_add(iface_totals.rx_bytes);
        tx_bytes = tx_bytes.saturating_add(iface_totals.tx_bytes);
        rx_packets = rx_packets.saturating_add(iface_totals.rx_packets);
        tx_packets = tx_packets.saturating_add(iface_totals.tx_packets);
        rx_dropped_packets = rx_dropped_packets.saturating_add(iface_totals.rx_dropped_packets);
        tx_dropped_packets = tx_dropped_packets.saturating_add(iface_totals.tx_dropped_packets);
    }

    Ok(NetworkTotals {
        rx_bytes,
        tx_bytes,
        rx_packets,
        tx_packets,
        rx_dropped_packets,
        tx_dropped_packets,
    })
}

fn parse_network_line_counters(counters: &str, iface: &str) -> Result<NetworkTotals> {
    let mut values = [0_u64; 16];
    let mut counter_count = 0_usize;

    for (index, raw_value) in counters.split_whitespace().enumerate() {
        let value = raw_value
            .parse::<u64>()
            .context("invalid network counter")?;
        if index < values.len() {
            values[index] = value;
        }
        counter_count += 1;
    }

    if counter_count < 16 {
        return Err(anyhow!(
            "expected 16 network counters for interface {iface}"
        ));
    }

    Ok(NetworkTotals {
        rx_bytes: values[0],
        tx_bytes: values[8],
        rx_packets: values[1],
        tx_packets: values[9],
        rx_dropped_packets: values[3],
        tx_dropped_packets: values[11],
    })
}
