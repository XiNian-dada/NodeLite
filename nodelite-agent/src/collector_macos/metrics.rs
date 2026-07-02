//! macOS metric aggregation built on top of syscall wrappers.

use std::collections::HashSet;
use std::ffi::CStr;
use std::mem;
use std::ptr;
use std::time::Instant;

use anyhow::{Result, anyhow};
use nodelite_proto::{DiskUsage, LoadAverage, MemoryUsage, percentage};
use tracing::warn;

use super::super::shared::{
    CpuSample, NetworkMetrics, NetworkSample, NetworkTotals, compute_network_metrics,
};
use super::syscall;

#[derive(Debug, Clone)]
pub(super) struct ObservedNetworkSample {
    pub(super) sample: NetworkSample,
    pub(super) signature: NetworkInterfaceSignature,
}

#[derive(Debug, Clone)]
pub(super) struct NetworkReading {
    pub(super) totals: NetworkTotals,
    pub(super) signature: NetworkInterfaceSignature,
}

/// macOS 的完整接口列表来自 `NET_RT_IFLIST2`,返回体会随 VPN/虚拟网卡变化。
/// 稳态下只保留 up/non-loopback 的 index,后续每轮用 `IFMIB_IFDATA` 轻量读取计数。
#[derive(Debug, Default)]
pub(super) struct NetworkInterfaceCache {
    pub(super) list_len: Option<usize>,
    pub(super) indices: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NetworkInterfaceSignature(Vec<String>);

impl NetworkInterfaceSignature {
    pub(super) fn empty() -> Self {
        Self(Vec::new())
    }

    pub(super) fn from_indices(indices: &[u16]) -> Self {
        let mut parts = indices
            .iter()
            .map(|index| format!("idx:{index}"))
            .collect::<Vec<_>>();
        parts.sort();
        Self(parts)
    }

    fn from_names(mut names: Vec<String>) -> Self {
        names.sort();
        Self(
            names
                .into_iter()
                .map(|name| format!("name:{name}"))
                .collect(),
        )
    }
}

pub(super) fn collect_cpu_sample() -> Result<CpuSample> {
    syscall::collect_cpu_sample()
}

pub(super) fn collect_load_average() -> Result<LoadAverage> {
    syscall::collect_load_average()
}

pub(super) fn collect_memory_usage() -> Result<MemoryUsage> {
    let memory = syscall::read_memory_statistics()?;
    let total_bytes = memory.total_bytes;
    let available_bytes =
        compute_available_memory_bytes(&memory.stats, memory.page_size, total_bytes);
    let used_bytes = total_bytes.saturating_sub(available_bytes);
    let (swap_total_bytes, swap_used_bytes) = syscall::read_swap_usage().unwrap_or((0, 0));

    Ok(MemoryUsage {
        total_bytes,
        used_bytes,
        available_bytes,
        swap_total_bytes,
        swap_used_bytes,
    })
}

/// 近似"可立即回收或可直接分配"的 RAM。
///
/// macOS 的 compressor 页本身并不是空闲页,但把它从 `inactive` 再减一次会把
/// 常见桌面负载下的 available 误压到 0,因此这里采用更保守、也更贴近 `vm_stat`
/// 直觉的估算:`free + inactive + purgeable`。
pub(super) fn compute_available_memory_bytes(
    stats: &libc::vm_statistics64,
    page_size: u64,
    total_bytes: u64,
) -> u64 {
    u64::from(stats.free_count)
        .saturating_add(u64::from(stats.inactive_count))
        .saturating_add(u64::from(stats.purgeable_count))
        .saturating_mul(page_size)
        .min(total_bytes)
}

pub(super) fn collect_disks() -> Result<Vec<DiskUsage>> {
    let mounts = syscall::mounted_filesystems()?;
    let mut seen_mounts = HashSet::new();
    let mut disks = Vec::new();

    for mount in &mounts {
        let device = syscall::c_chars_to_string(&mount.f_mntfromname)?;
        let mount_point = syscall::c_chars_to_string(&mount.f_mntonname)?;
        let fs_type = syscall::c_chars_to_string(&mount.f_fstypename)?;
        if ignored_filesystems().contains(&fs_type.as_str())
            || ignored_mount_point(&mount_point)
            || !seen_mounts.insert(mount_point.clone())
        {
            continue;
        }

        let block_size = u64::from(mount.f_bsize);
        let total_bytes = mount.f_blocks.saturating_mul(block_size);
        if total_bytes == 0 {
            continue;
        }
        let available_bytes = mount.f_bavail.saturating_mul(block_size);
        let used_bytes = total_bytes.saturating_sub(available_bytes);

        disks.push(DiskUsage {
            device,
            mount_point,
            fs_type,
            total_bytes,
            available_bytes,
            used_bytes,
            used_percent: percentage(used_bytes, total_bytes),
        });
    }

    disks.sort_by(|left, right| left.mount_point.cmp(&right.mount_point));
    Ok(disks)
}

fn ignored_filesystems() -> &'static [&'static str] {
    &["autofs", "devfs", "fdesc", "procfs", "volfs"]
}

fn ignored_mount_point(mount_point: &str) -> bool {
    matches!(
        mount_point,
        "/System/Volumes/Hardware"
            | "/System/Volumes/iSCPreboot"
            | "/System/Volumes/Preboot"
            | "/System/Volumes/Update"
            | "/System/Volumes/xarts"
    )
}

pub(super) fn collect_network_totals(cache: &mut NetworkInterfaceCache) -> Result<NetworkReading> {
    collect_network_totals_via_sysctl(cache).or_else(|error| {
        warn!(
            error = ?error,
            "failed to read macOS network counters via sysctl; falling back to getifaddrs",
        );
        cache.clear();
        collect_network_totals_via_ifaddrs()
    })
}

pub(super) fn compute_network_metrics_if_same_interfaces(
    previous: &ObservedNetworkSample,
    observed_at: Instant,
    current: NetworkTotals,
    current_signature: &NetworkInterfaceSignature,
) -> NetworkMetrics {
    if &previous.signature != current_signature {
        return NetworkMetrics::default();
    }
    compute_network_metrics(previous.sample, observed_at, current)
}

/// 优先用缓存接口 index + `IFMIB_IFDATA` 读取 64-bit 网卡计数。
/// 接口列表长度变化时再重读完整 `NET_RT_IFLIST2` buffer,刷新缓存。
fn collect_network_totals_via_sysctl(cache: &mut NetworkInterfaceCache) -> Result<NetworkReading> {
    let len = syscall::network_iflist2_len()?;

    if cache.can_sample_cached_indices(len) {
        match collect_cached_network_totals(cache) {
            Ok(totals) => {
                return Ok(NetworkReading {
                    totals,
                    signature: NetworkInterfaceSignature::from_indices(&cache.indices),
                });
            }
            Err(error) => {
                warn!(
                    error = ?error,
                    "failed to read cached macOS interface counters; refreshing interface list",
                );
                cache.clear();
            }
        }
    }

    let (totals, indices) = collect_network_totals_and_indices_via_iflist2(len)?;
    let signature = NetworkInterfaceSignature::from_indices(&indices);
    cache.list_len = Some(len);
    cache.indices = indices;
    Ok(NetworkReading { totals, signature })
}

fn collect_network_totals_and_indices_via_iflist2(len: usize) -> Result<(NetworkTotals, Vec<u16>)> {
    let buffer = syscall::read_network_iflist2(len)?;
    Ok(parse_network_totals_and_indices_from_iflist2(&buffer))
}

pub(super) fn parse_network_totals_and_indices_from_iflist2(
    buffer: &[u8],
) -> (NetworkTotals, Vec<u16>) {
    let mut seen_indices = HashSet::new();
    let mut indices = Vec::new();
    let mut rx_bytes = 0_u64;
    let mut tx_bytes = 0_u64;
    let mut rx_packets = 0_u64;
    let mut tx_packets = 0_u64;
    let mut rx_dropped_packets = 0_u64;
    let mut next = buffer.as_ptr();
    // SAFETY: `next` points at the start of `buffer`; adding exactly
    // `buffer.len()` yields the one-past-the-end pointer for the same slice.
    let end = unsafe { next.add(buffer.len()) };

    while next < end {
        let remaining = (end as usize).saturating_sub(next as usize);
        if remaining < mem::size_of::<libc::if_msghdr>() {
            break;
        }
        // SAFETY: The remaining byte count was checked for a full header.
        // `NET_RT_IFLIST2` buffers are byte streams, so use unaligned reads.
        let ifm = unsafe { ptr::read_unaligned(next.cast::<libc::if_msghdr>()) };
        let message_len = ifm.ifm_msglen as usize;
        if message_len == 0 || message_len > remaining {
            break;
        }

        if ifm.ifm_type == libc::RTM_IFINFO2 as u8 {
            if message_len < mem::size_of::<libc::if_msghdr2>() {
                break;
            }
            // SAFETY: The message declares enough bytes for `if_msghdr2`, and
            // the previous check keeps the read inside `buffer`.
            let ifm2 = unsafe { ptr::read_unaligned(next.cast::<libc::if_msghdr2>()) };
            let flags = ifm2.ifm_flags;
            let index = ifm2.ifm_index;
            if flags & libc::IFF_LOOPBACK == 0
                && flags & libc::IFF_UP != 0
                && seen_indices.insert(index)
            {
                indices.push(index);
                let data = ifm2.ifm_data;
                rx_bytes = rx_bytes.saturating_add(data.ifi_ibytes);
                tx_bytes = tx_bytes.saturating_add(data.ifi_obytes);
                rx_packets = rx_packets.saturating_add(data.ifi_ipackets);
                tx_packets = tx_packets.saturating_add(data.ifi_opackets);
                rx_dropped_packets = rx_dropped_packets.saturating_add(data.ifi_iqdrops);
            }
        }

        // SAFETY: `message_len` is non-zero and no larger than the remaining
        // bytes, so the next cursor stays within the same buffer or at `end`.
        next = unsafe { next.add(message_len) };
    }

    (
        NetworkTotals {
            rx_bytes,
            tx_bytes,
            rx_packets,
            tx_packets,
            rx_dropped_packets,
            tx_dropped_packets: 0,
        },
        indices,
    )
}

fn collect_cached_network_totals(cache: &NetworkInterfaceCache) -> Result<NetworkTotals> {
    let mut rx_bytes = 0_u64;
    let mut tx_bytes = 0_u64;
    let mut rx_packets = 0_u64;
    let mut tx_packets = 0_u64;
    let mut rx_dropped_packets = 0_u64;
    for index in &cache.indices {
        let data = syscall::collect_interface_data(*index)?;
        if data.ifmd_flags & libc::IFF_LOOPBACK as libc::c_uint != 0
            || data.ifmd_flags & libc::IFF_UP as libc::c_uint == 0
        {
            return Err(anyhow!("cached interface {index} changed flags"));
        }
        rx_bytes = rx_bytes.saturating_add(data.ifmd_data.ifi_ibytes);
        tx_bytes = tx_bytes.saturating_add(data.ifmd_data.ifi_obytes);
        rx_packets = rx_packets.saturating_add(data.ifmd_data.ifi_ipackets);
        tx_packets = tx_packets.saturating_add(data.ifmd_data.ifi_opackets);
        rx_dropped_packets = rx_dropped_packets.saturating_add(data.ifmd_data.ifi_iqdrops);
    }
    Ok(NetworkTotals {
        rx_bytes,
        tx_bytes,
        rx_packets,
        tx_packets,
        rx_dropped_packets,
        tx_dropped_packets: 0,
    })
}

impl NetworkInterfaceCache {
    pub(super) fn can_sample_cached_indices(&self, current_list_len: usize) -> bool {
        self.list_len == Some(current_list_len) && !self.indices.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.list_len = None;
        self.indices.clear();
    }
}

fn collect_network_totals_via_ifaddrs() -> Result<NetworkReading> {
    let addrs = syscall::get_ifaddrs()?;

    let mut seen_names = HashSet::new();
    let mut sampled_names = Vec::new();
    let mut rx_bytes = 0_u64;
    let mut tx_bytes = 0_u64;
    let mut rx_packets = 0_u64;
    let mut tx_packets = 0_u64;
    let mut rx_dropped_packets = 0_u64;
    let mut current = addrs.as_ptr();
    while !current.is_null() {
        // SAFETY: `current` starts at the head returned by `getifaddrs` and is
        // advanced through `ifa_next` while `addrs` keeps the list alive.
        let iface = unsafe { &*current };
        let address_family = if iface.ifa_addr.is_null() {
            None
        } else {
            // SAFETY: `ifa_addr` was checked non-null and belongs to the live
            // `getifaddrs` list guarded by `addrs`.
            Some(unsafe { (*iface.ifa_addr).sa_family as i32 })
        };
        if !iface.ifa_addr.is_null()
            && address_family == Some(libc::AF_LINK)
            && iface.ifa_flags & libc::IFF_LOOPBACK as u32 == 0
            && iface.ifa_flags & libc::IFF_UP as u32 != 0
            && !iface.ifa_data.is_null()
        {
            // SAFETY: macOS `getifaddrs` entries provide `ifa_name` as a
            // NUL-terminated interface name for the lifetime of the list.
            let name = unsafe { CStr::from_ptr(iface.ifa_name) }
                .to_string_lossy()
                .into_owned();
            if seen_names.insert(name.clone()) {
                sampled_names.push(name);
                // SAFETY: For AF_LINK entries with non-null `ifa_data`, macOS
                // stores a valid `if_data` counter block for this interface.
                let data = unsafe { &*(iface.ifa_data as *const libc::if_data) };
                rx_bytes = rx_bytes.saturating_add(u64::from(data.ifi_ibytes));
                tx_bytes = tx_bytes.saturating_add(u64::from(data.ifi_obytes));
                rx_packets = rx_packets.saturating_add(u64::from(data.ifi_ipackets));
                tx_packets = tx_packets.saturating_add(u64::from(data.ifi_opackets));
                rx_dropped_packets = rx_dropped_packets.saturating_add(u64::from(data.ifi_iqdrops));
            }
        }
        current = iface.ifa_next;
    }

    Ok(NetworkReading {
        totals: NetworkTotals {
            rx_bytes,
            tx_bytes,
            rx_packets,
            tx_packets,
            rx_dropped_packets,
            tx_dropped_packets: 0,
        },
        signature: NetworkInterfaceSignature::from_names(sampled_names),
    })
}
