//! Linux collector fixtures keep host filesystem state out of metric assertions.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;

use super::disks::FilesystemStats;
use super::{
    HostCollector, compute_cpu_usage, compute_network_metrics, parse_cpu_sample,
    parse_load_average, parse_memory_usage, parse_network_totals, real_statvfs,
};

/// RAII 临时目录:构造时创建唯一目录,析构时递归删除。
/// 即使断言 panic,Drop 仍会执行清理,避免临时目录泄漏。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create unique temp dir for collector test");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// statvfs 桩:返回固定容量,让磁盘采集完全脱离宿主机真实根文件系统。
fn mock_statvfs(_mount_point: &str) -> Result<FilesystemStats> {
    Ok(FilesystemStats {
        total_bytes: 100 * 1024 * 1024 * 1024,
        available_bytes: 40 * 1024 * 1024 * 1024,
        used_bytes: 60 * 1024 * 1024 * 1024,
    })
}

#[test]
fn parses_cpu_sample_and_usage() {
    let previous =
        parse_cpu_sample("cpu  100 0 50 400 10 0 0 0 0 0\n").expect("parse previous cpu sample");
    let current =
        parse_cpu_sample("cpu  160 0 70 430 20 0 0 0 0 0\n").expect("parse current cpu sample");
    let usage = compute_cpu_usage(previous, current);
    assert!(usage > 50.0 && usage < 70.0);
}

#[test]
fn parses_load_average() {
    let load = parse_load_average("0.11 0.22 0.33 1/100 12345\n").expect("parse load average line");
    assert_eq!(load.one, 0.11);
    assert_eq!(load.five, 0.22);
    assert_eq!(load.fifteen, 0.33);
}

#[test]
fn parses_memory_usage() {
    let memory = parse_memory_usage(
        "MemTotal:       1024 kB\nMemAvailable:    256 kB\nSwapTotal:       512 kB\nSwapFree:        128 kB\n",
    )
    .expect("parse meminfo block");
    assert_eq!(memory.total_bytes, 1024 * 1024);
    assert_eq!(memory.used_bytes, 768 * 1024);
    assert_eq!(memory.swap_used_bytes, 384 * 1024);
}

#[test]
fn parses_network_totals_and_rates() {
    let totals = parse_network_totals(
        "Inter-|   Receive                                                |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n eth0: 200 20 0 2 0 0 0 0 100 10 0 1 0 0 0 0\n lo: 50 5 0 0 0 0 0 0 50 5 0 0 0 0 0 0\n",
    )
    .expect("parse /proc/net/dev block");
    assert_eq!(totals.rx_bytes, 200);
    assert_eq!(totals.tx_bytes, 100);
    assert_eq!(totals.rx_packets, 20);
    assert_eq!(totals.tx_packets, 10);
    assert_eq!(totals.rx_dropped_packets, 2);
    assert_eq!(totals.tx_dropped_packets, 1);

    let previous = super::NetworkSample {
        observed_at: Instant::now() - Duration::from_secs(2),
        rx_bytes: 100,
        tx_bytes: 40,
        rx_packets: 10,
        tx_packets: 4,
        rx_dropped_packets: 0,
        tx_dropped_packets: 0,
    };
    let metrics = compute_network_metrics(previous, Instant::now(), totals);
    assert!(
        metrics
            .rx_bytes_per_sec
            .expect("rx rate present after two samples")
            > 40.0
    );
    assert!(
        metrics
            .tx_bytes_per_sec
            .expect("tx rate present after two samples")
            > 20.0
    );
    assert!(metrics.packet_loss_percent.is_some());
}

#[test]
fn real_statvfs_reports_missing_mount_errors() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let missing = std::env::temp_dir().join(format!(
        "nodelite-missing-statvfs-{}-{unique}",
        std::process::id()
    ));
    let error = match real_statvfs(missing.to_string_lossy().as_ref()) {
        Ok(_) => panic!("missing mount paths should surface statvfs errors"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("statvfs failed"));
}

#[test]
fn test_host_collector_with_mock_files() {
    let temp = TempDir::new("nodelite-collector-test");
    let root = temp.path();
    std::fs::create_dir_all(root.join("proc/sys/kernel")).expect("create mock proc/sys/kernel");
    std::fs::create_dir_all(root.join("proc/net")).expect("create mock proc/net");
    std::fs::create_dir_all(root.join("etc")).expect("create mock etc");

    // Write mock files
    std::fs::write(root.join("proc/uptime"), "3600.50 12345.67\n").expect("write mock proc/uptime");
    std::fs::write(root.join("proc/sys/kernel/hostname"), "mock-host\n")
        .expect("write mock hostname");
    std::fs::write(
        root.join("etc/os-release"),
        "PRETTY_NAME=\"Mock Linux OS\"\n",
    )
    .expect("write mock os-release");
    std::fs::write(root.join("proc/sys/kernel/osrelease"), "6.8.0-mock\n")
        .expect("write mock osrelease");
    std::fs::write(
        root.join("proc/cpuinfo"),
        "processor\t: 0\nmodel name\t: Mock CPU @ 3.0GHz\n\nprocessor\t: 1\nmodel name\t: Mock CPU @ 3.0GHz\n",
    )
    .expect("write mock cpuinfo");
    std::fs::write(
        root.join("proc/stat"),
        "cpu  100 0 50 400 10 0 0 0 0 0\ncpu0 50 0 25 200 5 0 0 0 0 0\n",
    )
    .expect("write mock proc/stat");
    std::fs::write(
        root.join("proc/net/dev"),
        "Inter-|   Receive                                                |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n eth0: 200 20 0 2 0 0 0 0 100 10 0 1 0 0 0 0\n",
    )
    .expect("write mock proc/net/dev");
    std::fs::write(root.join("proc/loadavg"), "0.15 0.30 0.45 1/100 12345\n")
        .expect("write mock loadavg");
    std::fs::write(
        root.join("proc/meminfo"),
        "MemTotal:       2097152 kB\nMemFree:         524288 kB\nMemAvailable:   1048576 kB\nSwapTotal:      1048576 kB\nSwapFree:        524288 kB\n",
    )
    .expect("write mock meminfo");
    std::fs::write(
        root.join("proc/mounts"),
        "/dev/vda1 / ext4 rw,relatime 0 0\ntmpfs /dev/shm tmpfs rw,nosuid,nodev 0 0\n",
    )
    .expect("write mock mounts");

    // 注入 statvfs 桩,确保磁盘采集只读 mock 数据,不触碰真实根文件系统。
    let mut collector = HostCollector::new_with_root(root.to_path_buf()).with_statvfs(mock_statvfs);
    let config = nodelite_proto::AgentConfig {
        node_id: "test-node".to_string(),
        node_label: "Test Node".to_string(),
        server: "ws://127.0.0.1:8080/ws".to_string(),
        token: "token".to_string(),
        connect_timeout_secs: 5,
        auth_timeout_secs: 20,
        send_timeout_secs: 20,
        inbound_timeout_secs: 90,
        report_interval_secs: 5,
        max_incoming_message_bytes: 65536,
        insecure_transport_warn_interval_secs: 900,
        tags: vec!["mock-tag".to_string()],
        hostname_override: None,
    };

    // Check identity collection
    let identity = collector
        .collect_identity(&config, "1.0.0")
        .expect("collect identity from mock root");
    assert_eq!(identity.node_id, "test-node");
    assert_eq!(identity.hostname, "mock-host");
    assert_eq!(identity.os, "Mock Linux OS");
    assert_eq!(identity.kernel_version, Some("6.8.0-mock".to_string()));
    assert_eq!(identity.cpu_model, Some("Mock CPU @ 3.0GHz".to_string()));
    assert_eq!(identity.cpu_cores, 2);
    assert_eq!(identity.agent_version, "1.0.0");
    assert_eq!(identity.tags, vec!["mock-tag".to_string()]);

    // Check snapshot collection (first collection has None rates)
    let snapshot1 = collector
        .collect_snapshot()
        .expect("collect snapshot from mock root");
    assert_eq!(snapshot1.uptime_secs, 3600);
    assert_eq!(snapshot1.load.one, 0.15);
    assert_eq!(snapshot1.load.five, 0.30);
    assert_eq!(snapshot1.load.fifteen, 0.45);
    assert_eq!(snapshot1.memory.total_bytes, 2097152 * 1024);
    assert_eq!(snapshot1.memory.available_bytes, 1048576 * 1024);
    assert_eq!(snapshot1.memory.used_bytes, 1048576 * 1024);
    assert_eq!(snapshot1.memory.swap_total_bytes, 1048576 * 1024);
    assert_eq!(snapshot1.memory.swap_used_bytes, 524288 * 1024);

    // Assert network totals
    assert_eq!(snapshot1.network.total_rx_bytes, 200);
    assert_eq!(snapshot1.network.total_tx_bytes, 100);
    assert_eq!(snapshot1.network.rx_bytes_per_sec, None);
    assert_eq!(snapshot1.network.tx_bytes_per_sec, None);
    assert_eq!(snapshot1.network.packet_loss_percent, None);

    // Disk usage comes entirely from the injected statvfs stub: the ext4 root is
    // reported, tmpfs is ignored, and the host's real `/` is never queried.
    assert_eq!(snapshot1.disks.len(), 1);
    let root_disk = &snapshot1.disks[0];
    assert_eq!(root_disk.device, "/dev/vda1");
    assert_eq!(root_disk.mount_point, "/");
    assert_eq!(root_disk.fs_type, "ext4");
    assert_eq!(root_disk.total_bytes, 100 * 1024 * 1024 * 1024);
    assert_eq!(root_disk.available_bytes, 40 * 1024 * 1024 * 1024);
    assert_eq!(root_disk.used_bytes, 60 * 1024 * 1024 * 1024);

    // 清理由 `TempDir` 的 Drop 负责,无需手动 remove_dir_all。
}
