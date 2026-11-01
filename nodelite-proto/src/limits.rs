//! Agent 与 Server 共用的快照容量上限。
//!
//! 两端必须引用同一组常量：Agent 在发送前按此裁剪，Server 默认按此清洗，
//! 否则大挂载点主机会因为两端上限不一致而被反复断开。

use std::collections::HashSet;

use crate::{NodeSnapshot, truncate_string_to_byte_boundary};

/// 单个快照最多上报的磁盘条目数，也是 Server `max_sanitized_disks` 的默认值。
pub const MAX_SNAPSHOT_DISKS: usize = 64;
/// 单个指标帧（解压后 JSON）的上限，也是 Server `max_message_bytes` 的默认值。
pub const MAX_METRICS_FRAME_BYTES: usize = 64 * 1024;
/// 磁盘字符串字段的上限，也是 Server `max_sanitized_string_bytes` 的默认值。
pub const MAX_SNAPSHOT_STRING_BYTES: usize = 256;
/// 为 `WireMessage` 外层标签预留的字节，快照本体必须在扣除后仍能放进帧上限。
const WIRE_ENVELOPE_RESERVE_BYTES: usize = 256;

/// 把快照裁剪到共享上限内，返回被丢弃的磁盘条目数。
///
/// 磁盘是快照里唯一随主机规模增长的字段，因此只裁剪磁盘；过长的字符串字段按字符边界截断，
/// 保留排序靠前的条目，
/// 让结果在相邻采样之间保持稳定。
pub fn fit_snapshot_to_wire_limits(
    snapshot: &mut NodeSnapshot,
) -> Result<usize, serde_json::Error> {
    let original = snapshot.disks.len();
    let mut seen_mounts = HashSet::new();
    // 先截断再去重：两条长路径可能只在上限之后才不同，截断后会变成同一挂载点。
    snapshot.disks.retain_mut(|disk| {
        truncate_string_to_byte_boundary(&mut disk.mount_point, MAX_SNAPSHOT_STRING_BYTES);
        truncate_string_to_byte_boundary(&mut disk.device, MAX_SNAPSHOT_STRING_BYTES);
        truncate_string_to_byte_boundary(&mut disk.fs_type, MAX_SNAPSHOT_STRING_BYTES);
        seen_mounts.insert(disk.mount_point.clone())
    });
    snapshot.disks.truncate(MAX_SNAPSHOT_DISKS);
    let budget = MAX_METRICS_FRAME_BYTES - WIRE_ENVELOPE_RESERVE_BYTES;
    let mut size = serde_json::to_vec(&*snapshot)?.len();
    while size > budget {
        let Some(disk) = snapshot.disks.pop() else {
            break;
        };
        // 加 1 计入数组分隔逗号；末条没有逗号造成的 1 字节偏差由信封预留吸收。
        size = size.saturating_sub(serde_json::to_vec(&disk)?.len() + 1);
    }
    Ok(original - snapshot.disks.len())
}

#[cfg(test)]
mod tests;
