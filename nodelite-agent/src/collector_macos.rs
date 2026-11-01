//! macOS 主机指标采集器:优先使用 libc / Mach 暴露的系统接口,
//! 在无法读取某个非关键指标时降级为保守值,避免整次采样直接失败。

#[path = "collector_macos/identity.rs"]
mod identity;
#[path = "collector_macos/metrics.rs"]
mod metrics;
#[path = "collector_macos/syscall.rs"]
mod syscall;

#[cfg(test)]
#[path = "collector_macos/tests.rs"]
mod tests;

use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use nodelite_proto::{AgentConfig, LoadAverage, NetworkCounters, NodeIdentity, NodeSnapshot};
use tracing::warn;

use self::metrics::{NetworkInterfaceCache, ObservedNetworkSample};
use super::shared::{
    CpuSample, NetworkMetrics, NetworkRateBaselines, NetworkSample, NetworkTotals,
    compute_cpu_usage,
};

/// 采集器状态:为了计算 CPU/网络的"差分速率",需要保留上一次的采样值。
pub struct HostCollector {
    previous_cpu: Option<CpuSample>,
    previous_network: Option<ObservedNetworkSample>,
    network_interfaces: NetworkInterfaceCache,
    network_rate_baselines: NetworkRateBaselines,
}

pub fn new_collector() -> HostCollector {
    HostCollector {
        previous_cpu: None,
        previous_network: None,
        network_interfaces: NetworkInterfaceCache::default(),
        network_rate_baselines: NetworkRateBaselines::default(),
    }
}

impl HostCollector {
    /// 组装节点身份。个别元数据拿不到时尽量回退,避免影响 Agent 启动。
    pub fn collect_identity(
        &self,
        config: &AgentConfig,
        agent_version: &str,
    ) -> Result<NodeIdentity> {
        identity::collect_identity(config, agent_version)
    }

    /// 采集一张完整快照。
    ///
    /// 首次调用时由于没有"上一次"的数据,`cpu_usage_percent` 与网络速率
    /// 都会返回 `None`,这是符合预期的初始状态。
    pub fn collect_snapshot(&mut self, ignored_filesystems: &[String]) -> Result<NodeSnapshot> {
        let cpu_sample = metrics::collect_cpu_sample()?;
        let cpu_usage_percent = self
            .previous_cpu
            .map(|previous| compute_cpu_usage(previous, cpu_sample));
        self.previous_cpu = Some(cpu_sample);

        let (network_totals, network_metrics) = match metrics::collect_network_totals(
            &mut self.network_interfaces,
        ) {
            Ok(reading) => self.observe_network_reading(reading),
            Err(error) => {
                // 累计计数是套餐账本的输入：伪造 0 会被下游当作计数器归零，所以读取失败时
                // 只能沿用上一次真实值；从未读到过则与 Linux 一样放弃这一帧。
                let Some(totals) = held_network_totals(self.previous_network.as_ref()) else {
                    return Err(error);
                };
                warn!(error = ?error, "failed to collect macOS network counters; holding last totals");
                (totals, Default::default())
            }
        };

        let load = match metrics::collect_load_average() {
            Ok(load) => load,
            Err(error) => {
                warn!(error = ?error, "failed to collect macOS load average; using zeros");
                LoadAverage {
                    one: 0.0,
                    five: 0.0,
                    fifteen: 0.0,
                }
            }
        };
        let memory = metrics::collect_memory_usage()?;
        let uptime_secs = syscall::read_uptime_secs()?;
        let disks = match metrics::collect_disks(ignored_filesystems) {
            Ok(disks) => disks,
            Err(error) => {
                warn!(error = ?error, "failed to collect macOS disks; using empty list");
                Vec::new()
            }
        };

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

    fn observe_network_reading(
        &mut self,
        reading: metrics::NetworkReading,
    ) -> (NetworkTotals, NetworkMetrics) {
        let observed_at = Instant::now();
        let network_totals = reading.totals;
        let network_signature = reading.signature;
        let interfaces_changed = self
            .previous_network
            .as_ref()
            .is_some_and(|previous| previous.signature != network_signature);
        let network_metrics = if let Some(previous) = &self.previous_network {
            metrics::compute_network_metrics_if_same_interfaces(
                previous,
                observed_at,
                network_totals,
                &network_signature,
            )
        } else {
            Default::default()
        };
        if interfaces_changed {
            self.network_rate_baselines.clear();
        }
        super::log_network_rate_anomalies(self.network_rate_baselines.observe(
            network_metrics.rx_bytes_per_sec,
            network_metrics.tx_bytes_per_sec,
        ));
        self.previous_network = Some(ObservedNetworkSample {
            sample: NetworkSample {
                observed_at,
                rx_bytes: network_totals.rx_bytes,
                tx_bytes: network_totals.tx_bytes,
                rx_packets: network_totals.rx_packets,
                tx_packets: network_totals.tx_packets,
                rx_dropped_packets: network_totals.rx_dropped_packets,
                tx_dropped_packets: network_totals.tx_dropped_packets,
            },
            signature: network_signature,
        });
        (network_totals, network_metrics)
    }
}

fn held_network_totals(previous: Option<&ObservedNetworkSample>) -> Option<NetworkTotals> {
    previous.map(|previous| NetworkTotals {
        rx_bytes: previous.sample.rx_bytes,
        tx_bytes: previous.sample.tx_bytes,
        rx_packets: previous.sample.rx_packets,
        tx_packets: previous.sample.tx_packets,
        rx_dropped_packets: previous.sample.rx_dropped_packets,
        tx_dropped_packets: previous.sample.tx_dropped_packets,
    })
}
